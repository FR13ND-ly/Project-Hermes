use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tracing::error;

#[derive(Debug)]
pub enum AppError {
    Validation(String),
    Auth(String),
    Permission(String),
    NotFound(String),
    Conflict(String),
    RateLimited(String),
    Infrastructure(String),
    Fatal(anyhow::Error),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Validation(msg) => write!(f, "Validation: {}", msg),
            AppError::Auth(msg) => write!(f, "Auth: {}", msg),
            AppError::Permission(msg) => write!(f, "Permission Denied: {}", msg),
            AppError::NotFound(msg) => write!(f, "Not Found: {}", msg),
            AppError::Conflict(msg) => write!(f, "Conflict: {}", msg),
            AppError::RateLimited(msg) => write!(f, "Rate Limited: {}", msg),
            AppError::Infrastructure(msg) => write!(f, "Infrastructure: {}", msg),
            AppError::Fatal(err) => write!(f, "Fatal Error: {}", err),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::Infrastructure(err.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // Reuse the request's correlation id so the error body, the access log and
        // the x-request-id response header all match (see middlewares::logger).
        let request_id = crate::middlewares::logger::current_request_id();

        let (status, code, message) = match self {
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, "VALIDATION_ERROR", msg),
            AppError::Auth(msg) => {
                tracing::warn!(%request_id, kind = "auth", "Authentication rejected: {}", msg);
                (StatusCode::UNAUTHORIZED, "AUTH_ERROR", msg)
            }
            AppError::Permission(msg) => {
                tracing::warn!(%request_id, kind = "permission", "Authorization denied: {}", msg);
                (StatusCode::FORBIDDEN, "PERMISSION_DENIED", msg)
            }
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            AppError::RateLimited(msg) => {
                tracing::warn!(%request_id, kind = "rate_limit", "Rate limit exceeded: {}", msg);
                (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED", msg)
            }

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
            sqlx::Error::Database(db_err) => {
                if db_err.code().as_deref() == Some("23505") {
                    let constraint_msg = match db_err.constraint() {
                        Some("unique_workspace_app_slug") => {
                            "O aplicație cu această denumire (slug) există deja în acest workspace."
                        }
                        Some("users_username_key") => {
                            "Numele de utilizator este deja folosit."
                        }
                        Some("users_email_key") => {
                            "Adresa de email este deja înregistrată."
                        }
                        Some("workspaces_slug_key") => {
                            "Un workspace cu această denumire (slug) există deja."
                        }
                        Some("unique_workspace_bucket_slug") => {
                            "Un bucket de stocare cu această denumire există deja în acest workspace."
                        }
                        Some("unique_route_path_per_project") => {
                            "Această rută serverless există deja în cadrul proiectului."
                        }
                        Some("unique_env_per_instance") => {
                            "O variabilă de mediu cu această cheie există deja în instanță."
                        }
                        _ => "Resursa duplicată încalcă o constrângere de unicitate.",
                    };
                    AppError::Conflict(constraint_msg.to_string())
                } else {
                    AppError::Fatal(sqlx::Error::Database(db_err).into())
                }
            }
            _ => AppError::Fatal(err.into()),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        AppError::Fatal(err)
    }
}