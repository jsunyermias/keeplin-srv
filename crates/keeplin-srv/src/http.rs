use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::header,
    middleware,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, AuthedUser},
    error::AppError,
    permissions::{resolve_note_access, resolve_notebook_access, Capabilities},
    state::AppState,
    store::{Note, NoteShare, Notebook, NotebookShare, ResourceMeta, Tag, User, UserDevice},
};

pub fn router(state: Arc<AppState>) -> Router {
    // Resource binary upload/download carries a raised body limit (metadata and
    // JSON routes keep axum's small default).
    let resource_data = Router::new()
        .route(
            "/api/resources/:id/data",
            get(get_resource_data).put(put_resource_data),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes));

    let protected = Router::new()
        // Aggregate counters are operational data (deployment size, live activity), so they
        // require a valid token rather than being world-readable (issue #22). Operators who
        // want stricter isolation should bind this behind an admin network/proxy.
        .route("/api/metrics", get(metrics))
        .route("/api/devices", post(create_device).get(list_devices))
        .route("/api/devices/:id", axum::routing::delete(delete_device))
        .route("/api/notes", post(create_note).get(list_notes))
        .route(
            "/api/notes/:id",
            get(get_note).patch(update_note).delete(delete_note),
        )
        .route("/api/notes/:id/share", post(create_share).get(list_shares))
        .route(
            "/api/notes/:id/share/:user_id",
            axum::routing::delete(delete_share),
        )
        .route("/api/notes/:id/transfer", post(transfer_ownership))
        .route("/api/notes/:id/history", get(note_history))
        .route("/api/notes/:id/export", get(export_note))
        .route("/api/import", post(import_note))
        // Domain entities the server materialises from the relay (read side for
        // cold rehydration / queries; writes still arrive over `/api/sync`).
        .route("/api/notebooks", get(list_notebooks))
        .route(
            "/api/notebooks/:id/share",
            post(create_notebook_share).get(list_notebook_shares),
        )
        .route(
            "/api/notebooks/:id/share/:user_id",
            axum::routing::delete(delete_notebook_share),
        )
        .route("/api/notebooks/:id/transfer", post(transfer_notebook))
        .route("/api/notebooks/:id/history", get(notebook_history))
        .route("/api/tags", get(list_tags))
        .route("/api/resources", get(list_resources))
        .route("/api/notes/:id/tags", get(list_note_tags))
        .merge(resource_data)
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_mw));

    // Everything except `/health` sits behind the per-IP rate limiter, so
    // orchestrator liveness probes are never throttled.
    let limited = Router::new()
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .merge(protected)
        // Collaborative editing channel (design §7): token in the
        // Authorization header (preferred) or `?token=` (fallback).
        .route("/api/ws", get(crate::collab::handler))
        // Device sync relay for keeplin-core's DbBackend.
        .route("/api/sync", get(crate::sync::handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::ratelimit::rate_limit_mw,
        ));

    Router::new()
        .route("/health", get(health))
        .merge(limited)
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
    let user = match state.store.get_user_by_email(&body.email).await? {
        Some(user) => user,
        None => {
            // Verify against a fixed dummy hash so a missing account costs the same Argon2
            // work as a wrong password — otherwise response timing reveals which emails have
            // accounts (issue #32). The result is discarded; the outcome is identical.
            let _ = auth::verify_password(&body.password, auth::dummy_password_hash());
            return Err(AppError::InvalidToken);
        }
    };

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

// ── Domain entities (server = source of truth; client DB is a cache) ─────────

/// Live notebooks of the authenticated user, for cold rehydration.
async fn list_notebooks(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<Notebook>>, AppError> {
    Ok(Json(state.store.list_notebooks(user.user_id).await?))
}

/// Live tags of the authenticated user.
async fn list_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<Tag>>, AppError> {
    Ok(Json(state.store.list_tags(user.user_id).await?))
}

/// Live resource metadata of the authenticated user. Binaries are fetched
/// separately via `GET /api/resources/:id/data`.
async fn list_resources(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<ResourceMeta>>, AppError> {
    Ok(Json(state.store.list_resources(user.user_id).await?))
}

/// Live tag ids attached to a note.
async fn list_note_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(note_id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, AppError> {
    Ok(Json(
        state.store.list_note_tag_ids(user.user_id, note_id).await?,
    ))
}

/// Download a resource's binary payload. The bytes are opaque (encrypted by the
/// client), so the content type is generic; the client already has the real
/// MIME type from the resource metadata.
async fn get_resource_data(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    if !state.store.resource_owned_by(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    let data = state
        .store
        .get_resource_blob(id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], data))
}

/// Upload (or replace) a resource's binary payload out-of-band. The resource
/// metadata must already exist for this user (it arrives over `/api/sync`); the
/// body is the raw bytes, capped by `MAX_UPLOAD_BYTES`.
async fn put_resource_data(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.store.resource_owned_by(id, user.user_id).await? {
        return Err(AppError::NotFound);
    }
    let limit = state.config.max_user_storage_bytes;
    if limit > 0 {
        // Count every other blob of this user plus the incoming one; replacing
        // a resource's blob is measured by its new size, not double-counted.
        let others = state
            .store
            .user_blob_bytes_excluding(user.user_id, id)
            .await?;
        if others + body.len() as i64 > limit {
            return Err(AppError::QuotaExceeded(format!(
                "storage limit reached ({limit} bytes)"
            )));
        }
    }
    state.store.put_resource_blob(id, &body).await?;
    Ok(Json(serde_json::json!({ "ok": true, "size": body.len() })))
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
    let limit = state.config.max_notes_per_user;
    if limit > 0 {
        let count = state.store.count_live_notes_for_user(user.user_id).await?;
        if count >= limit {
            return Err(AppError::QuotaExceeded(format!(
                "note limit reached ({limit})"
            )));
        }
    }
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
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden);
    }
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
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_write() {
        return Err(AppError::Forbidden);
    }
    // keeplin-core models the Inbox as the nil UUID (`ordering::INBOX_ID`); this server
    // models it as NULL. Map nil → NULL so a client mirroring an Inbox note performs a
    // move *out* of any notebook (shares untouched) instead of naming a notebook that
    // cannot exist — which would 404 on the destination check below.
    let notebook_id = match body.notebook_id {
        Some(Some(nb)) if nb.is_nil() => Some(None),
        other => other,
    };
    let patch = crate::store::NotePatch {
        title: body.title,
        notebook_id,
        is_todo: body.is_todo,
        todo_due: body.todo_due,
        todo_completed: body.todo_completed,
    };
    let moved_into = match &patch.notebook_id {
        // A move into a real notebook adopts that notebook's grants (destructive cascade).
        // A move to the Inbox (null) leaves the note's own shares untouched.
        Some(Some(nb)) if note.notebook_id != Some(*nb) => Some(*nb),
        _ => None,
    };
    // A move adopts the destination notebook's grants (destructive cascade), which both
    // discloses the note to that notebook's members and replaces the note's own shares.
    // So the mover needs `write` on the destination notebook too, not just on the note;
    // an unknown destination is `NotFound`. Moving *out* (to the Inbox) needs no
    // destination check.
    if let Some(nb) = moved_into {
        let nb_access = resolve_notebook_access(&state.store, nb, user.user_id).await?;
        if !nb_access.can_write() {
            return Err(AppError::Forbidden);
        }
    }
    let note = state
        .store
        .update_note_meta(id, &patch)
        .await?
        .ok_or(AppError::NotFound)?;
    if let Some(nb) = moved_into {
        state.store.apply_notebook_shares_to_note(id, nb).await?;
    }
    Ok(Json(note))
}

async fn delete_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    // Only the owner may delete the note (design §9.3).
    if !access.can_delete() {
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
    /// The capability bitmask to grant (see `permissions::Capabilities`). Implied bits are
    /// expanded server-side, and the grant is capped to the granter's own capabilities.
    capabilities: i32,
}

async fn create_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShareBody>,
) -> Result<Json<NoteShare>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_share_write() {
        return Err(AppError::Forbidden);
    }
    let requested = Capabilities::from_bits(body.capabilities);
    if requested.bits() == 0 {
        return Err(AppError::BadRequest(
            "capabilities must be non-empty".into(),
        ));
    }
    // No privilege escalation: you cannot grant a capability you do not hold yourself.
    if requested.bits() & access.caps.bits() != requested.bits() {
        return Err(AppError::Forbidden);
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
        .create_or_update_share(id, target.id, requested.bits())
        .await?;
    Ok(Json(share))
}

async fn list_shares(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NoteShare>>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.caps.can_share_read() {
        return Err(AppError::Forbidden);
    }
    Ok(Json(state.store.list_shares(id).await?))
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
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    // A `share_write` grantee can revoke anyone; anyone can remove themselves.
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state.store.delete_share(note_id, target_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Debug, Deserialize)]
struct TransferBody {
    /// The new owner, by id or email.
    user_id: Option<Uuid>,
    user_email: Option<String>,
}

/// `POST /api/notes/:id/transfer` — hand ownership to another user. Owner-only; ownership is
/// separate from the capability grants and survives the transfer (the old owner keeps no
/// implicit access unless separately shared).
async fn transfer_ownership(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_transfer_ownership() {
        return Err(AppError::Forbidden);
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
    // The new owner no longer needs a share row; drop any so their access is unambiguous.
    state.store.delete_share(id, target.id).await?;
    let note = state
        .store
        .set_note_owner(id, target.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}

// ── Notebook permissions (Front B stage 1b) ─────────────────────────────────────

/// Resolve a share/transfer target from `{user_id | user_email}` to a `User`.
async fn resolve_target(
    state: &AppState,
    user_id: Option<Uuid>,
    user_email: &Option<String>,
) -> Result<User, AppError> {
    match (user_id, user_email) {
        (Some(uid), _) => state.store.get_user_by_id(uid).await?,
        (None, Some(email)) => state.store.get_user_by_email(email).await?,
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)
}

async fn create_notebook_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CreateShareBody>,
) -> Result<Json<NotebookShare>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.can_share_write() {
        return Err(AppError::Forbidden);
    }
    let requested = Capabilities::from_bits(body.capabilities);
    if requested.bits() == 0 {
        return Err(AppError::BadRequest(
            "capabilities must be non-empty".into(),
        ));
    }
    if requested.bits() & access.caps.bits() != requested.bits() {
        return Err(AppError::Forbidden);
    }
    let target = resolve_target(&state, body.user_id, &body.user_email).await?;
    let owner = state
        .store
        .notebook_owner(id)
        .await?
        .ok_or(AppError::NotFound)?;
    if target.id == owner {
        return Err(AppError::BadRequest("owner already has access".into()));
    }
    // The share write cascades onto the notebook's notes inside the store call.
    let share = state
        .store
        .create_or_update_notebook_share(id, target.id, requested.bits())
        .await?;
    Ok(Json(share))
}

async fn list_notebook_shares(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<NotebookShare>>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.caps.can_share_read() {
        return Err(AppError::Forbidden);
    }
    Ok(Json(state.store.list_notebook_shares(id).await?))
}

async fn delete_notebook_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((notebook_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let access = resolve_notebook_access(&state.store, notebook_id, user.user_id).await?;
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    // Revocation re-cascades to the notebook's notes inside the store call.
    state
        .store
        .delete_notebook_share(notebook_id, target_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// `POST /api/notebooks/:id/transfer` — hand notebook ownership to another user (owner-only),
/// then re-cascade the notebook's grants so child notes reflect the new profile.
async fn transfer_notebook(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    Json(body): Json<TransferBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
    if !access.can_transfer_ownership() {
        return Err(AppError::Forbidden);
    }
    let target = resolve_target(&state, body.user_id, &body.user_email).await?;
    state
        .store
        .set_notebook_owner(id, target.id)
        .await?
        .ok_or(AppError::NotFound)?;
    state.store.delete_notebook_share(id, target.id).await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "owner_id": target.id }),
    ))
}

// ── History (Front D stage 2) ────────────────────────────────────────────────
//
// The server journal is the durable, cross-device change record; these endpoints expose it
// as version history so a fresh device (whose local journal is empty) can still show and
// revert through past versions. Scoped to the **caller's own journal** — that is where every
// change of their devices lands, and it makes the endpoints safe by construction (no
// cross-user reads). Snapshots are returned exactly as pushed: client-encrypted fields stay
// ciphertext and are decrypted client-side.

/// `?limit=` — version-count cap. Defaults to 100, hard-capped at 10 000 (the client's
/// revert scan bound).
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
}

const HISTORY_DEFAULT_LIMIT: u32 = 100;
const HISTORY_MAX_LIMIT: u32 = 10_000;

async fn entity_history(
    state: &AppState,
    user_id: Uuid,
    kind: crate::store::HistoryKind,
    id: Uuid,
    q: &HistoryQuery,
) -> Result<Vec<crate::store::EntityVersionRow>, AppError> {
    let limit = q
        .limit
        .filter(|l| *l > 0)
        .unwrap_or(HISTORY_DEFAULT_LIMIT)
        .min(HISTORY_MAX_LIMIT);
    // The age dimension of retention: even where pruning has not caught up yet, do not
    // serve history older than the configured journal retention window.
    let not_before = (state.config.retention_days > 0)
        .then(|| chrono::Utc::now() - chrono::Duration::days(state.config.retention_days as i64));
    state
        .store
        .entity_history(user_id, kind, id, limit as i64, not_before)
        .await
}

/// `GET /api/notes/:id/history` — past versions of a note from the caller's journal,
/// newest first. `entity` is the opaque snapshot the device pushed; `null` for a tombstone.
async fn note_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    Ok(Json(
        entity_history(
            &state,
            user.user_id,
            crate::store::HistoryKind::Note,
            id,
            &q,
        )
        .await?,
    ))
}

/// `GET /api/notebooks/:id/history` — past versions of a notebook. See [`note_history`].
async fn notebook_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    Ok(Json(
        entity_history(
            &state,
            user.user_id,
            crate::store::HistoryKind::Notebook,
            id,
            &q,
        )
        .await?,
    ))
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
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
    if !access.can_read() {
        return Err(AppError::Forbidden);
    }
    let body = materialize_body(&state, id).await?;
    Ok(Json(ExportResponse {
        id: note.id,
        title: note.title,
        body,
    }))
}
