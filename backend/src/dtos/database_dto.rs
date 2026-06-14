use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::database_model::{DbType, DbStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDatabaseRequest {
    pub project_id: Uuid,
    pub app_instance_id: Option<Uuid>,
    pub name: String,
    pub r#type: DbType,
    pub version: Option<String>,
    pub cpu_limit: Option<i32>,
    pub memory_limit_mb: Option<i64>,
    pub is_external: Option<bool>,
    pub external_port: Option<i32>,
    /// Publish the connection string into the project env pool (default: true).
    #[serde(default)]
    pub publish_to_env: Option<bool>,
    /// Override the suggested env key for the published connection string.
    #[serde(default)]
    pub env_key: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub app_instance_id: Option<Uuid>,
    pub name: String,
    pub r#type: DbType,
    pub version: String,
    pub db_user: String,
    pub db_name: String,
    pub container_name: String,
    pub internal_port: i32,
    pub is_external: bool,
    pub external_port: Option<i32>,
    pub status: DbStatus,
    pub connection_url: String,
}