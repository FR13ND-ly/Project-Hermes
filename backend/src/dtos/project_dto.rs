use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
}

/// Cloudflare / Ingress settings for a project. The API token is a secret: an
/// empty/omitted token leaves the stored value unchanged.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectSettingsRequest {
    pub cloudflare_api_token: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettingsResponse {
    pub cloudflare_zone_id: Option<String>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
    /// The token itself is never returned; only whether one is configured.
    pub has_cloudflare_token: bool,
}

#[derive(Debug, Serialize)]
pub struct ProjectResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ProjectDetailResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub total_memory_mb: i32,
    pub total_storage_gb: i32,
    pub component_count: i32,
    pub created_at: DateTime<Utc>,
}