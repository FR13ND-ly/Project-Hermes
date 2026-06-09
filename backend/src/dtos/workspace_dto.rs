use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateWorkspaceRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWorkspaceRequest {
    pub name: Option<String>,
    pub max_memory_mb: Option<i32>,
    pub max_storage_gb: Option<i32>,
    pub cloudflare_api_token: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub max_memory_mb: i32,
    pub max_storage_gb: i32,
    pub cloudflare_api_token: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceUsageResponse {
    pub workspace_id: Uuid,
    pub max_memory_mb: i32,
    pub used_memory_mb: i32,
    pub max_storage_gb: i32,
    pub used_storage_gb: i32,
}