use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFunctionRequest {
    pub name: String,
    pub code: Option<String>,
    pub method: String,
    pub route_path: String,
    pub memory_limit_mb: Option<i32>,
    pub runtime: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFunctionRequest {
    pub name: Option<String>,
    pub code: Option<String>,
    pub method: Option<String>,
    pub route_path: Option<String>,
    pub memory_limit_mb: Option<i32>,
    pub env_variables: Option<serde_json::Value>,
    pub assigned_domain: Option<Option<String>>,
    pub runtime: Option<String>,
    pub inherit_project_envs: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessBuildResponse {
    pub id: uuid::Uuid,
    pub status: String,
    pub image_tag: Option<String>,
    pub duration_sec: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionResponse {
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
    pub runtime: String,
    pub inherit_project_envs: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
