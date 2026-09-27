//! JSON API errors. Internal failures are logged server-side and answered with
//! a generic message, so database or file details never reach a client.
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "sign in required")
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }
    pub fn internal(error: impl std::fmt::Display) -> Self {
        let _ = error; // Never log credential-bearing transport or database details.
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Eplyx could not complete this request",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error": self.message}))).into_response()
    }
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        Self::internal(error)
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(error: deadpool_postgres::PoolError) -> Self {
        Self::internal(error)
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(error: anyhow::Error) -> Self {
        Self::internal(format!("{error:#}"))
    }
}
