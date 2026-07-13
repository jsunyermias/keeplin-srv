use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

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

impl AppError {
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

    /// The message sent to the client. Internal failures (database, unexpected
    /// internal errors) are collapsed to a generic string so their detail —
    /// which can name tables/columns/constraints — is not leaked in the
    /// response; the full error is logged server-side instead (issue #46).
    /// Caller-facing variants keep their specific, safe message.
    fn client_message(&self) -> String {
        match self {
            AppError::Database(_) | AppError::Internal(_) => "internal error".to_string(),
            other => other.to_string(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Log the full detail (including the sqlx/internal message) for operators,
        // but never put it in the client body.
        if status.is_server_error() {
            tracing::error!(error = %self, "request failed");
        }
        let body = Json(json!({ "error": self.client_message() }));
        (status, body).into_response()
    }
}
