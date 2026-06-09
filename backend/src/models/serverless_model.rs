use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessFunction {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub code: String,
    pub method: String,
    pub route_path: String,
    pub memory_limit_mb: i32,
    pub env_variables: serde_json::Value,
    pub status: String,
    pub assigned_domain: Option<String>,
    pub build_logs: Option<String>,
    pub external_port: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
