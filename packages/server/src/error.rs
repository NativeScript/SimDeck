use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Native(String),
    #[error("{0}")]
    Internal(String),
    /// The feature exists in SimDeck but this build cannot provide it on the
    /// current host platform.
    #[error("{0}")]
    Unsupported(String),
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

impl AppError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::BadRequest(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn native(message: impl Into<String>) -> Self {
        Self::Native(message.into())
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Native(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Unsupported(_) => StatusCode::NOT_IMPLEMENTED,
        };
        let message = self.to_string();
        (status, Json(ErrorBody { error: &message })).into_response()
    }
}
