use std::sync::Arc;

use axum::{
    extract::{Path, State},
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, AuthedUser},
    error::AppError,
    permissions::resolve_role,
    state::AppState,
    store::{Note, NoteShare, User, UserDevice},
};

pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/devices", post(create_device).get(list_devices))
        .route("/api/devices/:id", axum::routing::delete(delete_device))
        .route("/api/notes", post(create_note).get(list_notes))
        .route(
            "/api/notes/:id",
            get(get_note).patch(update_note).delete(delete_note),
        )
        .route("/api/notes/:id/share", post(create_share))
        .route(
            "/api/notes/:id/share/:user_id",
            axum::routing::delete(delete_share),
        )
        .route("/api/notes/:id/export", get(export_note))
        .route("/api/import", post(import_note))
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_mw));

    Router::new()
        .route("/health", get(health))
        .route("/api/metrics", get(metrics))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .merge(protected)
        // Collaborative editing channel (design §7): auth token in the query.
        .route("/api/ws", get(crate::collab::handler))
        // Device sync relay for keeplin-core's DbBackend.
        .route("/api/sync", get(crate::sync::handler))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

/// Aggregate operational counters: row counts plus live session/connection
/// numbers. No per-user data.
async fn metrics(State(state): State<Arc<AppState>>) -> Result<Json<serde_json::Value>, AppError> {
    let (users, notes, lines, tombstones) = state.store.counts().await?;
    let (collab_sessions, collab_connections) = state.collab.stats().await;
    let relay_users = state.hub.live_users().await;
    Ok(Json(serde_json::json!({
        "users": users,
        "notes": notes,
        "lines": lines,
        "line_tombstones": tombstones,
        "collab_sessions": collab_sessions,
        "collab_connections": collab_connections,
        "relay_live_users": relay_users,
    })))
}

// ── Auth ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct RegisterBody {
    email: String,
    password: String,
    /// Shown to other participants in collaborative sessions. Defaults to the
    /// part of the email before the '@'.
    display_name: Option<String>,
}

#[derive(Debug, serde::Serialize)]
struct RegisterResponse {
    user: User,
}

async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Result<Json<RegisterResponse>, AppError> {
    if body.password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let display_name = body
        .display_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| body.email.split('@').next().unwrap_or_default().to_string());
    let hash = auth::hash_password(&body.password)?;
    let user = state
        .store
        .create_user(&body.email, &hash, &display_name)
        .await?;
    Ok(Json(RegisterResponse { user }))
}

#[derive(Debug, Deserialize)]
struct LoginBody {
    email: String,
    password: String,
    device_name: String,
}

#[derive(Debug, serde::Serialize)]
struct LoginResponse {
    /// Device token: paste it into keeplin-daemon's `auth_token` config field.
    /// One login (one token) per device — the relay uses the device identity
    /// inside the token to know what each device has already received.
    token: String,
    device_id: Uuid,
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, AppError> {
    let user = state
        .store
        .get_user_by_email(&body.email)
        .await?
        .ok_or(AppError::InvalidToken)?;

    if !auth::verify_password(&body.password, &user.password_hash)? {
        return Err(AppError::InvalidToken);
    }

    let device = state
        .store
        .create_device(user.id, &body.device_name)
        .await?;

    let token = auth::create_token(
        user.id,
        device.id,
        &user.email,
        &state.config.jwt_secret,
        state.config.token_ttl_days,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse {
        token,
        device_id: device.id,
    }))
}

// ── Devices ──────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateDeviceBody {
    device_name: String,
}

#[derive(Debug, serde::Serialize)]
struct CreateDeviceResponse {
    token: String,
    device_id: Uuid,
    device_name: String,
}

/// Register an additional device for the authenticated user and return its
/// own token (equivalent to a fresh login without re-sending the password).
async fn create_device(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<CreateDeviceBody>,
) -> Result<Json<CreateDeviceResponse>, AppError> {
    let device = state
        .store
        .create_device(user.user_id, &body.device_name)
        .await?;
    let token = auth::create_token(
        user.user_id,
        device.id,
        &user.email,
        &state.config.jwt_secret,
        state.config.token_ttl_days,
    )
    .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(CreateDeviceResponse {
        token,
        device_id: device.id,
        device_name: device.device_name,
    }))
}

/// Revoke one of the caller's devices. Its token stops working immediately
/// on REST and on both WebSocket channels.
async fn delete_device(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.store.delete_device(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn list_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<UserDevice>>, AppError> {
    let devices = state.store.list_devices_by_user(user.user_id).await?;
    Ok(Json(devices))
}

// ── Notes ────────────────────────────────────────────────────────────────────

/// Materialise a note's body for non-collaborative reads (design §3.4): the
/// live lines, in order, joined with '\n'.
async fn materialize_body(state: &AppState, note_id: Uuid) -> Result<String, AppError> {
    let order = state
        .store
        .get_note_order(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let lines = state.store.list_lines(note_id).await?;
    let by_id: std::collections::HashMap<Uuid, _> = lines.into_iter().map(|l| (l.id, l)).collect();
    let body = order
        .order
        .iter()
        .filter_map(|id| by_id.get(id))
        .filter(|line| line.deleted_at.is_none())
        .map(|line| line.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(body)
}

#[derive(Debug, serde::Serialize)]
struct NoteResponse {
    #[serde(flatten)]
    note: Note,
    body: String,
}

#[derive(Debug, Deserialize)]
struct CreateNoteBody {
    /// Optional client-supplied id, so a daemon uploading a local note keeps
    /// the same note id on the server. 409 if it already exists.
    id: Option<Uuid>,
    #[serde(default = "default_title")]
    title: String,
}

fn default_title() -> String {
    "Untitled note".into()
}

async fn create_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<CreateNoteBody>,
) -> Result<Json<Note>, AppError> {
    let note = state
        .store
        .create_note(body.id, &body.title, user.user_id)
        .await?;
    Ok(Json(note))
}

async fn list_notes(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<Note>>, AppError> {
    let notes = state.store.list_notes_for_user(user.user_id).await?;
    Ok(Json(notes))
}

async fn get_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<NoteResponse>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;
    let body = materialize_body(&state, id).await?;
    Ok(Json(NoteResponse { note, body }))
}

/// Deserialize a present field (even an explicit `null`) as `Some(value)`,
/// so `PATCH` can distinguish "leave unchanged" (absent) from "clear" (null).
fn present<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize)]
struct UpdateNoteBody {
    title: Option<String>,
    #[serde(default, deserialize_with = "present")]
    notebook_id: Option<Option<Uuid>>,
    is_todo: Option<bool>,
    #[serde(default, deserialize_with = "present")]
    todo_due: Option<Option<chrono::DateTime<chrono::Utc>>>,
    #[serde(default, deserialize_with = "present")]
    todo_completed: Option<Option<chrono::DateTime<chrono::Utc>>>,
}

async fn update_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateNoteBody>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;
    if !role.can_write() {
        return Err(AppError::Forbidden);
    }
    let patch = crate::store::NotePatch {
        title: body.title,
        notebook_id: body.notebook_id,
        is_todo: body.is_todo,
        todo_due: body.todo_due,
        todo_completed: body.todo_completed,
    };
    let note = state
        .store
        .update_note_meta(id, &patch)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}

async fn delete_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;
    // Only the owner may delete the note (design §9.3).
    if !role.can_share() {
        return Err(AppError::Forbidden);
    }
    let note = state
        .store
        .soft_delete_note(id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}

// ── Shares ───────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateShareBody {
    /// Either the target user's id or their email must be provided.
    user_id: Option<Uuid>,
    user_email: Option<String>,
    role: String,
}

async fn create_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShareBody>,
) -> Result<Json<NoteShare>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;
    if !role.can_share() {
        return Err(AppError::Forbidden);
    }
    if body.role != "editor" && body.role != "viewer" {
        return Err(AppError::BadRequest("role must be editor or viewer".into()));
    }
    let target = match (body.user_id, &body.user_email) {
        (Some(user_id), _) => state.store.get_user_by_id(user_id).await?,
        (None, Some(email)) => state.store.get_user_by_email(email).await?,
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)?;
    if target.id == note.owner_id {
        return Err(AppError::BadRequest("owner already has access".into()));
    }
    let share = state
        .store
        .create_or_update_share(id, target.id, &body.role)
        .await?;
    Ok(Json(share))
}

async fn delete_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((note_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let note = state
        .store
        .get_note(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;
    // The owner can revoke anyone; anyone can remove themselves.
    if !role.can_share() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state.store.delete_share(note_id, target_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── Import / export (design §10) ─────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ImportBody {
    title: String,
    body: String,
}

#[derive(Debug, serde::Serialize)]
struct ImportResponse {
    note_id: Uuid,
    line_count: usize,
}

/// Offline → server migration for one note: split the flat body on '\n' into
/// one versioned line per row, seeding each line's vector with the importer's
/// component.
async fn import_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ImportBody>,
) -> Result<Json<ImportResponse>, AppError> {
    let note = state
        .store
        .create_note(None, &body.title, user.user_id)
        .await?;
    // The vv actor is the device, same as ops on the collaborative channel.
    let writer = user.device_id.to_string();
    let now = chrono::Utc::now();
    let lines: Vec<&str> = body.body.split('\n').collect();

    let mut order = Vec::with_capacity(lines.len());
    let line_vv = keeplin_core::storage::note_log::VersionVector::from([(writer.clone(), 1u64)]);
    for content in &lines {
        let line_id = Uuid::new_v4();
        state
            .store
            .insert_line(line_id, note.id, content, &line_vv, &writer, now)
            .await?;
        order.push(line_id);
    }
    let order_vv = keeplin_core::storage::note_log::VersionVector::from([(
        writer.clone(),
        lines.len() as u64,
    )]);
    state
        .store
        .set_note_order(note.id, &order, &order_vv, &writer, now)
        .await?;

    Ok(Json(ImportResponse {
        note_id: note.id,
        line_count: lines.len(),
    }))
}

#[derive(Debug, serde::Serialize)]
struct ExportResponse {
    id: Uuid,
    title: String,
    body: String,
}

/// Server → offline migration for one note: the live lines joined with '\n'.
async fn export_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportResponse>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;
    let body = materialize_body(&state, id).await?;
    Ok(Json(ExportResponse {
        id: note.id,
        title: note.title,
        body,
    }))
}
