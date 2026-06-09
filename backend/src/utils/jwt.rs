use chrono::{Duration, Utc};
use jsonwebtoken::{encode, EncodingKey, Header};
use sha2::{Sha256, Digest};
use uuid::Uuid;

use crate::middlewares::auth_middleware::Claims;
use crate::models::user_model::User;
use crate::utils::error::AppError;

pub struct TokenBundle {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub refresh_expires_at: chrono::DateTime<Utc>,
    pub refresh_token_hash: String,
}

pub fn generate_token_bundle(user: &User, _old_secret_param: &str) -> Result<TokenBundle, AppError> {
    let now = Utc::now();
    
    // Extragem dinamic secretele și expirările direct din .env
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "hermes_default_secret_fallback_key_32_bytes_long!!".to_string());
        
    let access_expiry_secs: i64 = std::env::var("JWT_ACCESS_EXPIRY")
        .unwrap_or_else(|_| "900".to_string())
        .parse()
        .unwrap_or(900);

    let refresh_expiry_secs: i64 = std::env::var("JWT_REFRESH_EXPIRY")
        .unwrap_or_else(|_| "604800".to_string())
        .parse()
        .unwrap_or(604800);

    let access_expiry = now + Duration::seconds(access_expiry_secs);
    
    let claims = Claims {
        sub: user.id,
        username: user.username.clone(),
        email: user.email.clone(),
        status: user.status,
        is_super_admin: user.is_super_admin,
        current_workspace_id: user.current_workspace_id,
        exp: access_expiry.timestamp(),
    };

    let access_token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;

    let refresh_token = format!("{}.{}", Uuid::new_v4(), Uuid::new_v4());
    let refresh_expires_at = now + Duration::seconds(refresh_expiry_secs);

    let mut hasher = Sha256::new();
    hasher.update(refresh_token.as_bytes());
    let refresh_token_hash = format!("{:x}", hasher.finalize());

    Ok(TokenBundle {
        access_token,
        refresh_token,
        expires_in: access_expiry_secs,
        refresh_expires_at,
        refresh_token_hash,
    })
}