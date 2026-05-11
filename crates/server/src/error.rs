use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use ironplc_bridge::BridgeError;
use project::StoreError;

/// Unified API error → HTTP response.
#[derive(Debug)]
pub enum ApiError {
    NoProject,
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    Internal(String),
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound(p) => Self::NotFound(format!("project not found: {p}")),
            StoreError::AlreadyExists(p) => Self::Conflict(format!("already exists: {p}")),
            StoreError::InvalidName(n) => Self::BadRequest(format!("invalid name: {n}")),
            StoreError::AppNotFound(n) => Self::NotFound(format!("application '{n}' not found")),
            StoreError::DeviceNotFound(n) => Self::NotFound(format!("device '{n}' not found")),
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<BridgeError> for ApiError {
    fn from(e: BridgeError) -> Self {
        Self::BadRequest(e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            Self::NoProject => (StatusCode::CONFLICT, "no project open".to_string()),
            Self::NotFound(s) => (StatusCode::NOT_FOUND, s),
            Self::Conflict(s) => (StatusCode::CONFLICT, s),
            Self::BadRequest(s) => (StatusCode::BAD_REQUEST, s),
            Self::Internal(s) => (StatusCode::INTERNAL_SERVER_ERROR, s),
        };
        (status, body).into_response()
    }
}
