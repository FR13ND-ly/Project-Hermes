use serde::{Deserialize, Serialize};
use uuid::Uuid;

// --- Instance ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstanceRequest {
    pub name: String,
    pub runtime: Option<String>,
    pub memory_limit_mb: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstanceRequest {
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub memory_limit_mb: Option<i32>,
    pub assigned_domain: Option<Option<String>>,
    pub inherit_project_envs: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteResponse {
    pub id: Uuid,
    pub instance_id: Uuid,
    pub method: String,
    pub route_path: String,
    pub code: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub runtime: String,
    pub memory_limit_mb: i32,
    pub status: String,
    pub assigned_domain: Option<String>,
    pub external_port: Option<i32>,
    pub inherit_project_envs: bool,
    pub routes: Vec<RouteResponse>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// --- Routes ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteRequest {
    pub method: String,
    pub route_path: String,
    pub code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRouteRequest {
    pub method: Option<String>,
    pub route_path: Option<String>,
    pub code: Option<String>,
}

// --- Env (per instance) ---

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInstanceEnvRequest {
    pub key: String,
    pub value: String,
    pub is_secret: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceEnvResponse {
    pub id: Uuid,
    pub instance_id: Uuid,
    pub key: String,
    /// Present only for non-secret vars (secrets are never returned in plaintext).
    pub value: Option<String>,
    pub is_secret: bool,
}

// --- Builds ---

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerlessBuildResponse {
    pub id: uuid::Uuid,
    pub status: String,
    pub image_tag: Option<String>,
    pub duration_sec: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}
