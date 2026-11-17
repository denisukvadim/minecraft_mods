use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use utoipa::ToSchema;

/// Uniform JSON error body returned by every failing endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Stable, machine-readable error kind.
    pub error: String,
    /// Human-readable explanation.
    pub message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("authentication is required")]
    Unauthorized,
    #[error("permission denied")]
    Forbidden,
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    PayloadTooLarge(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<jsonwebtoken::errors::Error> for AppError {
    fn from(_: jsonwebtoken::errors::Error) -> Self {
        Self::Unauthorized
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "authentication is required".to_owned(),
            ),
            Self::Forbidden => (
                StatusCode::FORBIDDEN,
                "forbidden",
                "you do not have the required permission".to_owned(),
            ),
            Self::NotFound(resource) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{resource} not found"),
            ),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Validation(message) => (StatusCode::BAD_REQUEST, "validation_error", message),
            Self::PayloadTooLarge(message) => {
                (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
            }
            Self::Database(err) => {
                tracing::error!(error = %err, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error".to_owned(),
                )
            }
            Self::Io(err) => {
                tracing::error!(error = %err, "filesystem error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error".to_owned(),
                )
            }
            Self::Internal(err) => {
                tracing::error!(error = %err, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error".to_owned(),
                )
            }
        };

        (
            status,
            Json(ErrorResponse {
                error: error.to_owned(),
                message,
            }),
        )
            .into_response()
    }
}

/// Maps a PostgreSQL unique-violation to a 409 instead of leaking a 500.
pub fn map_unique_violation(err: sqlx::Error, message: &'static str) -> AppError {
    if err
        .as_database_error()
        .is_some_and(|db_error| db_error.is_unique_violation())
    {
        AppError::Conflict(message.to_owned())
    } else {
        AppError::Database(err)
    }
}
