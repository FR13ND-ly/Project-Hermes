use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::domain_model::{DomainRoutingType, DomainStatus};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddDomainRequest {
    pub fqdn: String,
    pub routing_type: DomainRoutingType,
    pub client_max_body_size: Option<i32>,
    pub is_ssl: Option<bool>,
    pub nginx_target_host: Option<String>,
    pub nginx_root_path: Option<String>,
    pub nginx_config_content: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainResponse {
    pub id: Uuid,
    pub fqdn: String,
    pub routing_type: DomainRoutingType,
    pub status: DomainStatus,
    pub client_max_body_size: i32,
    pub is_ssl: bool,
    pub nginx_config_content: Option<String>,
    pub cf_proxy_active: bool,
    pub nginx_target_host: Option<String>,
    pub nginx_root_path: Option<String>,
}