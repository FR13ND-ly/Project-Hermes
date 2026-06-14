use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "domain_routing_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DomainRoutingType {
    ReverseProxy,
    StaticHost,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, sqlx::Type, PartialEq, Eq)]
#[sqlx(type_name = "domain_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DomainStatus {
    PendingVerification,
    Active,
    Failed,
}

#[derive(Debug, FromRow, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Domain {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub fqdn: String,
    /// 'app' | 'serverless' | 'database' | 'custom'.
    pub target_type: String,
    /// The targeted resource id (app instance / function / database); null for custom.
    pub target_id: Option<Uuid>,
    pub routing_type: DomainRoutingType,
    pub client_max_body_size: i32,
    pub is_ssl: bool,
    pub status: DomainStatus,
    pub nginx_target_host: Option<String>,
    pub nginx_root_path: Option<String>,
    pub nginx_config_content: Option<String>,
    pub cloudflare_zone_id: Option<String>,
    pub cloudflare_record_id: Option<String>,
    pub cf_proxy_active: bool,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}