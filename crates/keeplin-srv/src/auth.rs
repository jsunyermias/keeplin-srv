use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Body,
    extract::{FromRequestParts, State},
    http::{request::Parts, Request},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::{error::AppError, state::AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid, // user_id
    pub device_id: Uuid,
    pub email: String,
    pub exp: usize,
}

#[derive(Debug, Clone)]
pub struct AuthedUser {
    pub user_id: Uuid,
    pub device_id: Uuid,
    pub email: String,
}

pub fn hash_password(password: &str) -> Result<String, AppError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(format!("password hash failed: {}", e)))?;
    Ok(hash.to_string())
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, AppError> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Internal(format!("invalid password hash: {}", e)))?;
    let argon2 = Argon2::default();
    Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
}

/// A valid Argon2 hash of a fixed dummy password, computed once. `login` verifies against
/// it when the email has no account so that a missing account and a wrong password take the
/// same time, closing the user-enumeration timing side-channel (issue #32).
pub fn dummy_password_hash() -> &'static str {
    use std::sync::OnceLock;
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        hash_password("timing-equalizer-not-a-real-password")
            .expect("hashing a fixed dummy password never fails")
    })
}

pub fn create_token(
    user_id: Uuid,
    device_id: Uuid,
    email: &str,
    secret: &str,
    ttl_days: i64,
) -> Result<String, jsonwebtoken::errors::Error> {
    let claims = Claims {
        sub: user_id,
        device_id,
        email: email.to_string(),
        exp: (chrono::Utc::now() + chrono::Duration::days(ttl_days)).timestamp() as usize,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
}

pub fn verify_token(token: &str, secret: &str) -> Result<AuthedUser, AppError> {
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .map_err(|e| {
        tracing::debug!(error = %e, "token verification failed");
        AppError::InvalidToken
    })?;

    Ok(AuthedUser {
        user_id: token_data.claims.sub,
        device_id: token_data.claims.device_id,
        email: token_data.claims.email,
    })
}

pub async fn auth_mw(
    state: State<Arc<AppState>>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let State(state) = state;
    let auth = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());
    let token = auth
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(AppError::MissingToken)?;
    let user = verify_token(token, &state.config.jwt_secret)?;
    // Deleting a device revokes its token immediately: the claim must still
    // reference a live device of the same user, not just carry a valid
    // signature.
    match state.store.get_device(user.device_id).await? {
        Some(device) if device.user_id == user.user_id => {}
        _ => return Err(AppError::InvalidToken),
    }
    req.extensions_mut().insert(user);
    Ok(next.run(req).await)
}

#[async_trait::async_trait]
impl<S> FromRequestParts<S> for AuthedUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthedUser>()
            .cloned()
            .ok_or(AppError::MissingToken)
    }
}
