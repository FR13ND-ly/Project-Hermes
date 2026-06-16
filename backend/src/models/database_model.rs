use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "db_type", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DbType {
    Postgres,
    Mysql,
    Redis,
    Mongodb,
}

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "db_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum DbStatus {
    Provisioning,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseService {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub app_instance_id: Option<Uuid>,
    pub name: String,
    pub r#type: DbType,
    pub version: String,
    pub db_user: String,
    pub db_password: String,
    pub db_password_nonce: Option<String>,
    pub db_name: String,
    pub container_name: String,
    pub internal_port: i32,
    pub is_external: bool,
    pub external_port: Option<i32>,
    pub status: DbStatus,
    pub cpu_limit: i32,
    pub memory_limit_mb: i64,
    pub storage_size_gb: i32,
    pub backup_enabled: bool,
    pub backup_count: i32,
    pub last_backup_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}