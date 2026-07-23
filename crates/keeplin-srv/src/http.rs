// md:Overview
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::header,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    auth::{self, AuthedUser},
    error::AppError,
    permissions::{resolve_note_access, resolve_notebook_access, Capabilities},
    state::AppState,
    store::{Note, NoteShare, NotebookShare, PageCursor, User, UserDevice},
};

// md:MAX_PAGE_LIMIT
const MAX_PAGE_LIMIT: i64 = 500;

// md:ListQuery
#[derive(Debug, Deserialize)]
struct ListQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

// md:impl ListQuery
impl ListQuery {
    // md:impl ListQuery > fn resolve
    fn resolve(&self) -> Result<(Option<i64>, Option<PageCursor>), AppError> {
        let limit = self.limit.map(|l| l.clamp(1, MAX_PAGE_LIMIT));
        let cursor = match self.cursor.as_deref() {
            Some(token) => Some(
                PageCursor::decode(token)
                    .ok_or_else(|| AppError::BadRequest("invalid cursor".into()))?,
            ),
            None => None,
        };
        Ok((limit, cursor))
    }
}

// md:fn paginated
fn paginated<T: Serialize>(
    items: Vec<T>,
    limit: Option<i64>,
    cursor_of: impl Fn(&T) -> PageCursor,
) -> Response {
    let next = match limit {
        Some(l) if items.len() as i64 >= l => items.last().map(|it| cursor_of(it).encode()),
        _ => None,
    };
    let mut resp = Json(items).into_response();
    if let Some(token) = next {
        if let Ok(value) = token.parse() {
            resp.headers_mut().insert("x-next-cursor", value);
        }
    }
    resp
}

// md:fn router
pub fn router(state: Arc<AppState>) -> Router {
    let resource_data = Router::new()
        .route(
            "/api/resources/:id/data",
            get(get_resource_data).put(put_resource_data),
        )
        .layer(DefaultBodyLimit::max(state.config.max_upload_bytes));

    let protected = Router::new()
        .route("/api/metrics", get(metrics))
        .route(
            "/api/devices",
            post(create_device)
                .get(list_devices)
                .delete(delete_all_devices),
        )
        .route("/api/devices/:id", axum::routing::delete(delete_device))
        .route("/api/account/password", post(change_password))
        .route("/api/account", axum::routing::delete(delete_account))
        .route("/api/account/verify/request", post(verify_request))
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

    let limited = Router::new()
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .route("/api/account/verify/confirm", post(verify_confirm))
        .route("/api/account/reset/request", post(reset_request))
        .route("/api/account/reset/confirm", post(reset_confirm))
        .merge(protected)
        .route("/api/ws", get(crate::collab::handler))
        .route("/api/sync", get(crate::sync::handler))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            crate::ratelimit::rate_limit_mw,
        ));

    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/version", get(version))
        .merge(limited)
        .with_state(state)
}

// md:PROTOCOL_VERSION
pub const PROTOCOL_VERSION: u32 = 1;

// md:fn compatible_with
pub fn compatible_with(client_protocol: u32) -> bool {
    client_protocol == PROTOCOL_VERSION
}

// md:CAPABILITIES
const CAPABILITIES: &[&str] = &[
    "history",
    "history_visibility",
    "resource_purge",
    "readiness",
    "account_management",
    "pagination",
    "email_flows",
];

// md:fn version
async fn version() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "keeplin-srv",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": PROTOCOL_VERSION,
        "capabilities": CAPABILITIES,
    }))
}

// md:fn health
async fn health() -> &'static str {
    "ok"
}

// md:fn ready
async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.store.ping().await {
        Ok(()) => (axum::http::StatusCode::OK, "ready"),
        Err(e) => {
            tracing::warn!(error = %e, "readiness check failed");
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "database unavailable",
            )
        }
    }
}

// md:MetricsQuery
#[derive(Debug, Deserialize)]
struct MetricsQuery {
    format: Option<String>,
}

// md:fn metrics
async fn metrics(
    State(state): State<Arc<AppState>>,
    Query(q): Query<MetricsQuery>,
) -> Result<Response, AppError> {
    let (users, notes, lines, tombstones) = state.store.counts().await?;
    let (collab_sessions, collab_connections) = state.collab.stats().await;
    let relay_users = state.hub.live_users().await;

    if q.format.as_deref() == Some("prometheus") {
        let body = format!(
            "# HELP keeplin_users Registered accounts (shared across replicas).\n\
             # TYPE keeplin_users gauge\n\
             keeplin_users {users}\n\
             # HELP keeplin_notes Live notes (shared across replicas).\n\
             # TYPE keeplin_notes gauge\n\
             keeplin_notes {notes}\n\
             # HELP keeplin_lines Live note lines (shared across replicas).\n\
             # TYPE keeplin_lines gauge\n\
             keeplin_lines {lines}\n\
             # HELP keeplin_line_tombstones Soft-deleted lines awaiting GC (shared across replicas).\n\
             # TYPE keeplin_line_tombstones gauge\n\
             keeplin_line_tombstones {tombstones}\n\
             # HELP keeplin_collab_sessions Live collaborative note sessions on this instance.\n\
             # TYPE keeplin_collab_sessions gauge\n\
             keeplin_collab_sessions {collab_sessions}\n\
             # HELP keeplin_collab_connections Live collaborative connections on this instance.\n\
             # TYPE keeplin_collab_connections gauge\n\
             keeplin_collab_connections {collab_connections}\n\
             # HELP keeplin_relay_live_users Users with a live relay connection on this instance.\n\
             # TYPE keeplin_relay_live_users gauge\n\
             keeplin_relay_live_users {relay_users}\n"
        );
        return Ok(([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response());
    }

    Ok(Json(serde_json::json!({
        "users": users,
        "notes": notes,
        "lines": lines,
        "line_tombstones": tombstones,
        "collab_sessions": collab_sessions,
        "collab_connections": collab_connections,
        "relay_live_users": relay_users,
    }))
    .into_response())
}

// md:fn normalize_email
fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

// md:fn is_valid_email
fn is_valid_email(email: &str) -> bool {
    let mut parts = email.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => {
            !local.is_empty()
                && domain.len() >= 3
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        _ => false,
    }
}

// md:RegisterBody
#[derive(Debug, Deserialize)]
struct RegisterBody {
    email: String,
    password: String,
    display_name: Option<String>,
}

// md:RegisterResponse
#[derive(Debug, serde::Serialize)]
struct RegisterResponse {
    user: User,
}

// md:fn register
async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> Result<Json<RegisterResponse>, AppError> {
    if !state.config.registration_enabled {
        return Err(AppError::Forbidden);
    }
    if body.password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let email = normalize_email(&body.email);
    if !is_valid_email(&email) {
        return Err(AppError::BadRequest("invalid email".into()));
    }
    let display_name = body
        .display_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| email.split('@').next().unwrap_or_default().to_string());
    let hash = auth::hash_password(&body.password)?;
    let user = state
        .store
        .create_user(&email, &hash, &display_name)
        .await?;
    if state.mailer.enabled() {
        if let Err(e) = send_flow_mail(&state, &user, crate::mail::MailKind::VerifyEmail).await {
            tracing::error!(error = %e, "verification mail on register failed");
        }
    }
    Ok(Json(RegisterResponse { user }))
}

// md:LoginBody
#[derive(Debug, Deserialize)]
struct LoginBody {
    email: String,
    password: String,
    device_name: String,
}

// md:LoginResponse
#[derive(Debug, serde::Serialize)]
struct LoginResponse {
    token: String,
    device_id: Uuid,
}

// md:fn login
async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, AppError> {
    let email = normalize_email(&body.email);

    let lockout_enabled = state.config.login_max_failures > 0;
    if lockout_enabled && state.store.login_locked(&email).await? {
        return Err(AppError::TooManyAttempts);
    }
    let record_failure = || async {
        if lockout_enabled {
            state
                .store
                .record_login_failure(
                    &email,
                    state.config.login_max_failures,
                    state.config.login_lockout_secs,
                )
                .await?;
        }
        Ok::<(), AppError>(())
    };

    let user = match state.store.get_user_by_email(&email).await? {
        Some(user) => user,
        None => {
            let _ = auth::verify_password(&body.password, auth::dummy_password_hash());
            record_failure().await?;
            return Err(AppError::InvalidToken);
        }
    };

    if !auth::verify_password(&body.password, &user.password_hash)? {
        record_failure().await?;
        return Err(AppError::InvalidToken);
    }

    if state.config.email_verification_required && user.email_verified_at.is_none() {
        return Err(AppError::BadRequest("email not verified".into()));
    }

    if lockout_enabled {
        state.store.clear_login_failures(&email).await?;
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

// md:CreateDeviceBody
#[derive(Debug, Deserialize)]
struct CreateDeviceBody {
    device_name: String,
}

// md:CreateDeviceResponse
#[derive(Debug, serde::Serialize)]
struct CreateDeviceResponse {
    token: String,
    device_id: Uuid,
    device_name: String,
}

// md:fn create_device
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

// md:fn delete_device
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

// md:fn list_devices
async fn list_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<UserDevice>>, AppError> {
    let devices = state.store.list_devices_by_user(user.user_id).await?;
    Ok(Json(devices))
}

// md:fn delete_all_devices
async fn delete_all_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let removed = state.store.delete_all_devices(user.user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true, "revoked": removed })))
}

// md:ChangePasswordBody
#[derive(Debug, Deserialize)]
struct ChangePasswordBody {
    current_password: String,
    new_password: String,
}

// md:fn change_password
async fn change_password(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ChangePasswordBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !auth::verify_password(&body.current_password, &stored.password_hash)? {
        return Err(AppError::InvalidToken);
    }
    let hash = auth::hash_password(&body.new_password)?;
    state.store.update_password(user.user_id, &hash).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:DeleteAccountBody
#[derive(Debug, Deserialize)]
struct DeleteAccountBody {
    password: String,
}

// md:fn delete_account
async fn delete_account(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<DeleteAccountBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if !auth::verify_password(&body.password, &stored.password_hash)? {
        return Err(AppError::InvalidToken);
    }
    state.store.delete_user(user.user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:fn send_flow_mail
async fn send_flow_mail(
    state: &AppState,
    user: &User,
    kind: crate::mail::MailKind,
) -> Result<(), AppError> {
    let (token, expires_at) = state
        .store
        .create_email_token(user.id, kind, state.config.email_token_ttl_secs)
        .await?;
    state
        .mailer
        .send(kind, &user.email, &user.display_name, &token, expires_at)
        .await
        .map_err(AppError::Internal)
}

// md:fn verify_request
async fn verify_request(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.mailer.enabled() {
        return Err(AppError::NotImplemented(
            "mail webhook not configured".into(),
        ));
    }
    let stored = state
        .store
        .get_user_by_id(user.user_id)
        .await?
        .ok_or(AppError::NotFound)?;
    if stored.email_verified_at.is_some() {
        return Ok(Json(
            serde_json::json!({ "ok": true, "already_verified": true }),
        ));
    }
    send_flow_mail(&state, &stored, crate::mail::MailKind::VerifyEmail).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:TokenBody
#[derive(Debug, Deserialize)]
struct TokenBody {
    token: String,
}

// md:fn verify_confirm
async fn verify_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<TokenBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    let user_id = state
        .store
        .consume_email_token(crate::mail::MailKind::VerifyEmail, &body.token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    state.store.mark_email_verified(user_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:ResetRequestBody
#[derive(Debug, Deserialize)]
struct ResetRequestBody {
    email: String,
}

// md:fn reset_request
async fn reset_request(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResetRequestBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.mailer.enabled() {
        return Err(AppError::NotImplemented(
            "mail webhook not configured".into(),
        ));
    }
    let email = normalize_email(&body.email);
    if let Some(user) = state.store.get_user_by_email(&email).await? {
        if let Err(e) = send_flow_mail(&state, &user, crate::mail::MailKind::PasswordReset).await {
            tracing::error!(error = %e, "password reset mail failed");
        }
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:ResetConfirmBody
#[derive(Debug, Deserialize)]
struct ResetConfirmBody {
    token: String,
    new_password: String,
}

// md:fn reset_confirm
async fn reset_confirm(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ResetConfirmBody>,
) -> Result<Json<serde_json::Value>, AppError> {
    if body.new_password.len() < 8 {
        return Err(AppError::BadRequest("password too short".into()));
    }
    let user_id = state
        .store
        .consume_email_token(crate::mail::MailKind::PasswordReset, &body.token)
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired token".into()))?;
    let hash = auth::hash_password(&body.new_password)?;
    state.store.update_password(user_id, &hash).await?;
    state.store.delete_all_devices(user_id).await?;
    if let Some(user) = state.store.get_user_by_id(user_id).await? {
        state.store.clear_login_failures(&user.email).await?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:fn list_notebooks
async fn list_notebooks(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = state
        .store
        .list_notebooks(user.user_id, limit, cursor)
        .await?;
    Ok(paginated(items, limit, |nb| {
        PageCursor::new(nb.created_at, nb.id)
    }))
}

// md:fn list_tags
async fn list_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = state.store.list_tags(user.user_id, limit, cursor).await?;
    Ok(paginated(items, limit, |t| {
        PageCursor::new(t.created_at, t.id)
    }))
}

// md:ResourceListFilter
#[derive(Debug, Deserialize)]
struct ResourceListFilter {
    #[serde(default)]
    note_id: Option<Uuid>,
}

// md:fn list_resources
async fn list_resources(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
    Query(f): Query<ResourceListFilter>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let items = match f.note_id {
        Some(note_id) => {
            state
                .store
                .list_resources_for_note(user.user_id, note_id, limit, cursor)
                .await?
        }
        None => {
            state
                .store
                .list_resources(user.user_id, limit, cursor)
                .await?
        }
    };
    Ok(paginated(items, limit, |r| {
        PageCursor::new(r.created_at, r.id)
    }))
}

// md:fn list_note_tags
async fn list_note_tags(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(note_id): Path<Uuid>,
) -> Result<Json<Vec<Uuid>>, AppError> {
    Ok(Json(
        state.store.list_note_tag_ids(user.user_id, note_id).await?,
    ))
}

// md:fn get_resource_data
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

// md:fn put_resource_data
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

// md:fn materialize_body
async fn materialize_body(state: &AppState, note_id: Uuid) -> Result<String, AppError> {
    let order = state
        .store
        .get_note_order(note_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let lines = state.store.list_lines(note_id).await?;
    let by_id: std::collections::HashMap<Uuid, _> = lines.into_iter().map(|l| (l.id, l)).collect();
    let live: Vec<&str> = order
        .order
        .iter()
        .filter_map(|id| by_id.get(id))
        .filter(|line| line.deleted_at.is_none())
        .map(|line| line.content.as_str())
        .collect();

    let cap = state.config.max_note_body_bytes;
    if cap > 0 {
        let separators = live.len().saturating_sub(1);
        let total = live.iter().map(|s| s.len()).sum::<usize>() + separators;
        if total > cap {
            return Err(AppError::PayloadTooLarge(format!(
                "note body is {total} bytes, exceeds the {cap}-byte limit"
            )));
        }
    }

    Ok(live.join("\n"))
}

// md:NoteResponse
#[derive(Debug, serde::Serialize)]
struct NoteResponse {
    #[serde(flatten)]
    note: Note,
    body: String,
}

// md:CreateNoteBody
#[derive(Debug, Deserialize)]
struct CreateNoteBody {
    id: Option<Uuid>,
    #[serde(default = "default_title")]
    title: String,
}

// md:fn default_title
fn default_title() -> String {
    "Untitled note".into()
}

// md:fn create_note
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

// md:fn list_notes
async fn list_notes(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Query(q): Query<ListQuery>,
) -> Result<Response, AppError> {
    let (limit, cursor) = q.resolve()?;
    let notes = state
        .store
        .list_notes_for_user(user.user_id, limit, cursor)
        .await?;
    Ok(paginated(notes, limit, |n| {
        PageCursor::new(n.updated_at, n.id)
    }))
}

// md:fn get_note
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

// md:fn present
fn present<'de, D, T>(de: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    T::deserialize(de).map(Some)
}

// md:UpdateNoteBody
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

// md:fn update_note
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
        Some(Some(nb)) if note.notebook_id != Some(*nb) => Some(*nb),
        _ => None,
    };
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

// md:fn delete_note
async fn delete_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
) -> Result<Json<Note>, AppError> {
    let note = state.store.get_note(id).await?.ok_or(AppError::NotFound)?;
    let access = resolve_note_access(&state.store, &note, user.user_id).await?;
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

// md:CreateShareBody
#[derive(Debug, Deserialize)]
struct CreateShareBody {
    user_id: Option<Uuid>,
    user_email: Option<String>,
    capabilities: i32,
}

// md:fn create_share
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
    if requested.bits() & access.caps.bits() != requested.bits() {
        return Err(AppError::Forbidden);
    }
    let target = match (body.user_id, &body.user_email) {
        (Some(user_id), _) => state.store.get_user_by_id(user_id).await?,
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
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

// md:fn list_shares
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

// md:fn delete_share
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
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state.store.delete_share(note_id, target_id).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:TransferBody
#[derive(Debug, Deserialize)]
struct TransferBody {
    user_id: Option<Uuid>,
    user_email: Option<String>,
}

// md:fn transfer_ownership
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
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)?;
    state.store.delete_share(id, target.id).await?;
    let note = state
        .store
        .set_note_owner(id, target.id)
        .await?
        .ok_or(AppError::NotFound)?;
    Ok(Json(note))
}

// md:fn resolve_target
async fn resolve_target(
    state: &AppState,
    user_id: Option<Uuid>,
    user_email: &Option<String>,
) -> Result<User, AppError> {
    match (user_id, user_email) {
        (Some(uid), _) => state.store.get_user_by_id(uid).await?,
        (None, Some(email)) => {
            state
                .store
                .get_user_by_email(&normalize_email(email))
                .await?
        }
        (None, None) => {
            return Err(AppError::BadRequest(
                "user_id or user_email required".into(),
            ))
        }
    }
    .ok_or(AppError::NotFound)
}

// md:fn create_notebook_share
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
    let share = state
        .store
        .create_or_update_notebook_share(id, target.id, requested.bits())
        .await?;
    Ok(Json(share))
}

// md:fn list_notebook_shares
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

// md:fn delete_notebook_share
async fn delete_notebook_share(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path((notebook_id, target_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let access = resolve_notebook_access(&state.store, notebook_id, user.user_id).await?;
    if !access.can_share_write() && target_id != user.user_id {
        return Err(AppError::Forbidden);
    }
    state
        .store
        .delete_notebook_share(notebook_id, target_id)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

// md:fn transfer_notebook
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

// md:HistoryQuery
#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
}

// md:History limits
const HISTORY_DEFAULT_LIMIT: u32 = 100;
const HISTORY_MAX_LIMIT: u32 = 10_000;

// md:fn history_versions
async fn history_versions(
    state: &AppState,
    kind: crate::store::HistoryKind,
    id: Uuid,
    q: &HistoryQuery,
    access_cutoff: Option<chrono::DateTime<chrono::Utc>>,
    user_scope: Option<Uuid>,
) -> Result<Vec<crate::store::EntityVersionRow>, AppError> {
    let limit = q
        .limit
        .filter(|l| *l > 0)
        .unwrap_or(HISTORY_DEFAULT_LIMIT)
        .min(HISTORY_MAX_LIMIT);
    let retention_cutoff = (state.config.retention_days > 0)
        .then(|| chrono::Utc::now() - chrono::Duration::days(state.config.retention_days as i64));
    state
        .store
        .entity_history(
            kind,
            id,
            limit as i64,
            retention_cutoff,
            access_cutoff,
            user_scope,
        )
        .await
}

// md:fn access_cutoff
fn access_cutoff(
    state: &AppState,
    access: &crate::permissions::Access,
    share_created_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    if state.config.history_since_access && !access.is_owner {
        share_created_at
    } else {
        None
    }
}

// md:fn note_history
async fn note_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    match state.store.get_note(id).await? {
        Some(note) => {
            let access = resolve_note_access(&state.store, &note, user.user_id).await?;
            if !access.can_read() {
                return Err(AppError::Forbidden);
            }
            let share = state.store.get_share(id, user.user_id).await?;
            let cutoff = access_cutoff(&state, &access, share.map(|s| s.created_at));
            Ok(Json(
                history_versions(
                    &state,
                    crate::store::HistoryKind::Note,
                    id,
                    &q,
                    cutoff,
                    None,
                )
                .await?,
            ))
        }
        None => Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Note,
                id,
                &q,
                None,
                Some(user.user_id),
            )
            .await?,
        )),
    }
}

// md:fn notebook_history
async fn notebook_history(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Path(id): Path<Uuid>,
    axum::extract::Query(q): axum::extract::Query<HistoryQuery>,
) -> Result<Json<Vec<crate::store::EntityVersionRow>>, AppError> {
    if state.store.notebook_owner(id).await?.is_some() {
        let access = resolve_notebook_access(&state.store, id, user.user_id).await?;
        if !access.can_read() {
            return Err(AppError::Forbidden);
        }
        let share = state.store.get_notebook_share(id, user.user_id).await?;
        let cutoff = access_cutoff(&state, &access, share.map(|s| s.created_at));
        Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Notebook,
                id,
                &q,
                cutoff,
                None,
            )
            .await?,
        ))
    } else {
        Ok(Json(
            history_versions(
                &state,
                crate::store::HistoryKind::Notebook,
                id,
                &q,
                None,
                Some(user.user_id),
            )
            .await?,
        ))
    }
}

// md:ImportBody
#[derive(Debug, Deserialize)]
struct ImportBody {
    title: String,
    body: String,
}

// md:ImportResponse
#[derive(Debug, serde::Serialize)]
struct ImportResponse {
    note_id: Uuid,
    line_count: usize,
}

// md:fn import_note
async fn import_note(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
    Json(body): Json<ImportBody>,
) -> Result<Json<ImportResponse>, AppError> {
    let note = state
        .store
        .create_note(None, &body.title, user.user_id)
        .await?;
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

// md:ExportResponse
#[derive(Debug, serde::Serialize)]
struct ExportResponse {
    id: Uuid,
    title: String,
    body: String,
}

// md:fn export_note
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

// md:mod tests
#[cfg(test)]
mod tests {
    use super::*;

    // md:mod tests > fn protocol_compatibility_is_exact_match
    #[test]
    fn protocol_compatibility_is_exact_match() {
        assert!(compatible_with(PROTOCOL_VERSION));
        assert!(!compatible_with(PROTOCOL_VERSION + 1));
        assert!(!compatible_with(0));
    }
}
