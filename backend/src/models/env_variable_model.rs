use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub app_instance_id: Uuid,
    pub key: String,
    pub encrypted_value: String,
    pub nonce: String,
    pub is_secret: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
