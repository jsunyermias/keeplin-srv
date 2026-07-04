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
    store::{Note, NoteShare, User},
};

pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/devices", post(create_device))
        .route("/api/notes", post(create_note).get(list_notes))
        .route("/api/notes/:id", get(get_note).patch(update_note).delete(delete_note))
        .route("/api/notes/:id/shares", post(create_share))
        .route("/api/notes/:id/shares/:user_id", get(get_share).delete(delete_share))
        .route("/api/notes/:id/export", get(export_note))
        .route("/api/import", post(import_note))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::auth_mw,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .merge(protected)
        .route("/api/ws", get(crate::websocket::handler))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

// Auth

#[derive(Debug, Deserialize)]
struct RegisterBody {
    email: String,
    password: String,
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
    let hash = auth::hash_password(&body.password)?;
    let user = state.store.create_user(&body.email, &hash).await?;
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

    let token = auth::create_token(user.id, device.id, &user.email, &state.config.jwt_secret)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(LoginResponse { token, device_id: device.id }))
}

// Devices

#[derive(Debug, Deserialize)]
struct CreateDeviceBody {
    device_name: String,
}

#[derive(Debug, serde::Serialize)]
struct DeviceResponse {
    id: Uuid,
    device_name: String,
}

async fn create_device(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<CreateDeviceBody>,
) -> Result<Json<DeviceResponse>, AppError> {
    let device = state.store.create_device(user.user_id, &body.device_name).await?;
    Ok(Json(DeviceResponse {
        id: device.id,
        device_name: device.device_name,
    }))
}

// Notes

#[derive(Debug, Deserialize)]
struct CreateNoteBody {
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
    let note = state.store.create_note(&body.title, user.user_id).await?;
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
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;
    Ok(Json(note))
}

#[derive(Debug, Deserialize)]
struct UpdateNoteBody {
    title: String,
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
    let note = state
        .store
        .update_note_title(id, &body.title)
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
    if !role.can_share() {
        return Err(AppError::Forbidden);
    }
    let note = state.store.soft_delete_note(id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(note))
}

// Shares

#[derive(Debug, Deserialize)]
struct CreateShareBody {
    user_email: String,
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
    let target = state
        .store
        .get_user_by_email(&body.user_email)
        .await?
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

async fn get_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((note_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<NoteShare>, AppError> {
    let note = state.store.get_note(note_id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;
    let share = state.store.get_share(note_id, user_id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(share))
}

async fn delete_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((note_id, user_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let note = state.store.get_note(note_id).await?.ok_or(AppError::NotFound)?;
    let role = resolve_role(&state.store, &note, user.user_id).await?;
    if !role.can_share() && user_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state.store.delete_share(note_id, user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// Import / export

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

async fn import_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ImportBody>,
) -> Result<Json<ImportResponse>, AppError> {
    let note = state.store.create_note(&body.title, user.user_id).await?;
    let lines: Vec<&str> = body.body.split('\n').collect();
    let base_vv = keeplin_core::storage::note_log::VersionVector::new();
    let mut previous_line_id: Option<Uuid> = None;
    for content in &lines {
        let (line, _) = crate::lines::insert_line(
            &state.store,
            note.id,
            previous_line_id,
            content,
            &base_vv,
            &user.device_id.to_string(),
        )
        .await?;
        previous_line_id = Some(line.id);
    }
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

async fn export_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<ExportResponse>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    resolve_role(&state.store, &note, user.user_id).await?;
    let rows = state.store.list_note_lines_active(id).await?;
    let body = rows
        .into_iter()
        .map(|(_, line)| line.content)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(Json(ExportResponse {
        id: note.id,
        title: note.title,
        body,
    }))
}
