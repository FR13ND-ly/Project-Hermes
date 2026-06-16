use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct GitCredential {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub provider: String,
    pub host: String,
    pub label: String,
    pub username: Option<String>,
    pub encrypted_token: String,
    pub nonce: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub skip_tls_verify: bool,
}
