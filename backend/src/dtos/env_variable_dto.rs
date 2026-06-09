use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::models::env_variable_model::EnvScope;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetEnvRequest {
    pub project_id: Option<Uuid>,
    pub app_instance_id: Option<Uuid>,
    pub key: String,
    pub value: String,
    pub scope: Option<EnvScope>,
    pub is_secret: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvResponse {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub app_instance_id: Option<Uuid>,
    pub key: String,
    pub value: Option<String>,
    pub scope: EnvScope,
    pub is_secret: bool,
}