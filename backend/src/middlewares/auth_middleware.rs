use axum::{
    async_trait,
    extract::FromRequestParts,
    http::request::Parts,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::models::user_model::UserStatus;
use crate::utils::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub username: String,
    pub email: String,
    pub status: UserStatus,
    pub is_super_admin: bool,
    pub current_workspace_id: Option<Uuid>,
    pub exp: i64,
}

pub struct AuthenticatedUser(pub Claims);

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let token = if let Some(auth_header) = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
        {
            if !auth_header.starts_with("Bearer ") {
                return Err(AppError::Auth("Invalid Authorization header format".to_string()));
            }
            auth_header[7..].to_string()
        } else if let Some(query) = parts.uri.query() {
            let token_param = query.split('&')
                .find(|part| part.starts_with("token="))
                .map(|part| part[6..].to_string());
            
            token_param.ok_or_else(|| AppError::Auth("Missing Authorization token in query parameter".to_string()))?
        } else {
            return Err(AppError::Auth("Missing Authorization header or token query parameter".to_string()));
        };
        let jwt_secret = crate::config::secrets::jwt_secret();

        let token_data = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| AppError::Auth("Invalid or expired token".to_string()))?;

        if token_data.claims.status == UserStatus::Suspended {
            return Err(AppError::Permission("This account has been suspended".to_string()));
        }

        let path = parts.uri.path();
        if token_data.claims.status == UserStatus::PendingVerification && path != "/api/v1/auth/activate" {
            return Err(AppError::Permission("Account activation required".to_string()));
        }

        Ok(AuthenticatedUser(token_data.claims))
    }
}

pub async fn require_auth(
    _user: AuthenticatedUser,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, AppError> {
    Ok(next.run(request).await)
}

pub async fn enforce_super_admin(
    AuthenticatedUser(claims): AuthenticatedUser,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Super Admin privileges required".to_string()));
    }
    Ok(next.run(request).await)
}