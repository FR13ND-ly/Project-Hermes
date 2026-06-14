use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub cloudflare_api_token: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub ingress_ip: Option<String>,
    pub base_domain: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "resource_type", rename_all = "lowercase")]
pub enum ResourceType {
    Application,
    Database,
    Storage,
}

#[derive(Debug, Clone, FromRow)]
pub struct ProjectResource {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub resource_type: ResourceType,
    pub memory_mb: i32,
    pub storage_gb: i32,
    pub created_at: DateTime<Utc>,
}