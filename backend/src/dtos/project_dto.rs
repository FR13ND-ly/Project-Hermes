use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateProjectRequest {
    pub name: String,
    /// Optional workspace Cloudflare credential to associate at creation time.
    #[serde(default)]
    pub cloudflare_credential_id: Option<Uuid>,
}

/// Cloudflare / Ingress settings for a project. The Cloudflare token now lives on a
/// workspace credential; the project just references one (null = none).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProjectSettingsRequest {
    #[serde(default)]
    pub cloudflare_credential_id: Option<Uuid>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSettingsResponse {
    /// The associated workspace Cloudflare credential, if any.
    pub cloudflare_credential_id: Option<Uuid>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
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