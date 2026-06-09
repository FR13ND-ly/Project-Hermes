use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "app_instance_type", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum AppInstanceType {
    Production,
    Staging,
    Preview,
}

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "app_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Building,
    Running,
    Stopped,
    Failed,
    Crashed,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct App {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    pub git_repository: String,
    pub build_command: Option<String>,
    pub start_command: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub git_subpath: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppInstance {
    pub id: Uuid,
    pub app_id: Uuid,
    pub branch_name: String,
    pub instance_type: AppInstanceType,
    pub status: AppStatus,
    pub internal_port: i32,
    pub assigned_domain: Option<String>,
    pub container_name: String,
    pub cpu_limit: i32,
    pub memory_limit_mb: i64,
    pub meta_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub external_port: Option<i32>,
}