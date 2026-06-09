use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "env_scope", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum EnvScope {
    Production,
    Staging,
    Preview,
    All,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Option<Uuid>,
    pub app_instance_id: Option<Uuid>,
    pub key: String,
    pub encrypted_value: String,
    pub nonce: String,
    pub scope: EnvScope,
    pub is_secret: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}