use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::domain_model::{DomainRoutingType, DomainStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDomainRequest {
    pub fqdn: String,
    /// 'app' | 'serverless' | 'database' | 'custom'. Defaults to 'custom'.
    #[serde(default = "default_target_type")]
    pub target_type: String,
    /// The targeted resource id (app instance / function / database).
    #[serde(default)]
    pub target_id: Option<Uuid>,
    /// Only used for target_type='custom'; otherwise derived from the target.
    #[serde(default)]
    pub routing_type: Option<DomainRoutingType>,
    pub client_max_body_size: Option<i32>,
    pub is_ssl: Option<bool>,
    pub nginx_target_host: Option<String>,
    pub nginx_root_path: Option<String>,
    pub nginx_config_content: Option<String>,
}

fn default_target_type() -> String {
    "custom".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainResponse {
    pub id: Uuid,
    pub fqdn: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub target_name: Option<String>,
    pub routing_type: DomainRoutingType,
    pub status: DomainStatus,
    pub client_max_body_size: i32,
    pub is_ssl: bool,
    pub nginx_config_content: Option<String>,
    pub cf_proxy_active: bool,
    pub nginx_target_host: Option<String>,
    pub nginx_root_path: Option<String>,
}
