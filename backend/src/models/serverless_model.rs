use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// A serverless instance = one container/Knative service. It owns the runtime,
/// memory, domain, env and build; routes live in `serverless_routes`.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessInstance {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub runtime: String,
    pub memory_limit_mb: i32,
    pub status: String,
    pub assigned_domain: Option<String>,
    pub external_port: Option<i32>,
    pub current_image_tag: Option<String>,
    pub inherit_project_envs: bool,
    pub build_logs: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single route inside an instance: HTTP method + path + handler code.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessRoute {
    pub id: Uuid,
    pub instance_id: Uuid,
    pub method: String,
    pub route_path: String,
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ServerlessEnvVariable {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub instance_id: Uuid,
    pub key: String,
    pub encrypted_value: String,
    pub nonce: String,
    pub is_secret: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessBuild {
    pub id: Uuid,
    pub instance_id: Uuid,
    pub workspace_id: Uuid,
    pub status: String,
    pub logs: String,
    pub image_tag: Option<String>,
    pub duration_sec: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
