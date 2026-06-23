use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// A BaaS end user. Identity is a per-app unique `identifier` + password; nothing
/// else is stored on the account (any extra data the app wants on the token is
/// supplied per-request as custom claims, not persisted here).
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppUser {
    pub id: Uuid,
    pub baas_id: Uuid,
    pub identifier: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub status: String,
    pub last_login: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppUserRole {
    pub id: Uuid,
    pub baas_id: Uuid,
    pub app_user_id: Uuid,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppApiKey {
    pub id: Uuid,
    pub baas_id: Uuid,
    pub name: String,
    #[serde(skip_serializing)]
    pub key_hash: String,
    pub key_prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}
