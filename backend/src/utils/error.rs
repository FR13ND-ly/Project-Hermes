use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::error;
use uuid::Uuid;

#[derive(Debug)]
pub enum AppError {
    Validation(String),
    Auth(String),
    Permission(String),
    NotFound(String),
    Conflict(String),
    Infrastructure(String),
    Fatal(anyhow::Error), 
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = Uuid::new_v4().to_string();

        let (status, code, message) = match self {
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, "VALIDATION_ERROR", msg),
            AppError::Auth(msg) => (StatusCode::UNAUTHORIZED, "AUTH_ERROR", msg),
            AppError::Permission(msg) => (StatusCode::FORBIDDEN, "PERMISSION_DENIED", msg),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            
            AppError::Infrastructure(msg) => {
                error!(%request_id, "Infrastructure error: {}", msg);
                (
                    StatusCode::SERVICE_UNAVAILABLE, 
                    "INFRASTRUCTURE_ERROR", 
                    "Cluster is temporarily busy. Try again in a minute.".to_string()
                )
            },
            AppError::Fatal(err) => {
                error!(%request_id, "Fatal internal error: {:#}", err);
                (
                    StatusCode::INTERNAL_SERVER_ERROR, 
                    "INTERNAL_ERROR", 
                    "A technical error occurred. Our team is notified.".to_string()
                )
            }
        };

        let body = Json(json!({
            "error": {
                "code": code,
                "message": message,
                "request_id": request_id
            }
        }));

        (status, body).into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match err {
            sqlx::Error::RowNotFound => AppError::NotFound("The requested resource was not found.".to_string()),
            _ => AppError::Fatal(err.into()),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Fatal(err)
    }
}