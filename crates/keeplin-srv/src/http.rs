use std::sync::Arc;

use axum::{
    extract::State,
    middleware,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    auth::{self, AuthedUser},
    error::AppError,
    state::AppState,
    store::{User, UserDevice},
};

pub fn router(state: Arc<AppState>) -> Router {
    let protected = Router::new()
        .route("/api/devices", post(create_device).get(list_devices))
        .layer(middleware::from_fn_with_state(state.clone(), auth::auth_mw));

    Router::new()
        .route("/health", get(health))
        .route("/api/register", post(register))
        .route("/api/login", post(login))
        .merge(protected)
        .route("/api/sync", get(crate::sync::handler))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

// ── Auth ─────────────────────────────────────────────────────────────────────

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

    let device = state.store.create_device(user.id, &body.device_name).await?;

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

async fn list_devices(
    State(state): State<Arc<AppState>>,
    user: AuthedUser,
) -> Result<Json<Vec<UserDevice>>, AppError> {
    let devices = state.store.list_devices_by_user(user.user_id).await?;
    Ok(Json(devices))
}
