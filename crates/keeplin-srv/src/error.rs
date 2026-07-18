// md:Overview
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

// md:AppError
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("missing token")]
    MissingToken,

    #[error("invalid token")]
    InvalidToken,

    #[error("not found")]
    NotFound,

    #[error("forbidden")]
    Forbidden,

    #[error("conflict")]
    Conflict,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    #[error("payload too large: {0}")]
    PayloadTooLarge(String),

    #[error("too many attempts; try again later")]
    TooManyAttempts,

    #[error("not implemented: {0}")]
    NotImplemented(String),

    #[error("internal error: {0}")]
    Internal(String),
}

// md:impl AppError
impl AppError {
    // md:impl AppError > fn status
    fn status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;
        match self {
            AppError::Database(sqlx::Error::RowNotFound) => StatusCode::NOT_FOUND,
            AppError::Database(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::MissingToken | AppError::InvalidToken => StatusCode::UNAUTHORIZED,
            AppError::NotFound => StatusCode::NOT_FOUND,
            AppError::Forbidden => StatusCode::FORBIDDEN,
            AppError::Conflict => StatusCode::CONFLICT,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::QuotaExceeded(_) => StatusCode::INSUFFICIENT_STORAGE,
            AppError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            AppError::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
            AppError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // md:impl AppError > fn client_message
    fn client_message(&self) -> String {
        match self {
            AppError::Database(_) | AppError::Internal(_) => "internal error".to_string(),
            other => other.to_string(),
        }
    }
}

// md:impl IntoResponse for AppError
impl IntoResponse for AppError {
    // md:impl IntoResponse for AppError > fn into_response
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        let body = Json(json!({ "error": self.client_message() }));
        (status, body).into_response()
    }
}
