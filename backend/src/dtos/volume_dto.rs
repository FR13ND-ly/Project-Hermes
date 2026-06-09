use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVolumeRequest {
    pub app_id: Uuid,
    pub name: String,
    pub container_path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VolumeResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub name: String,
    pub container_path: String,
    pub host_path: String,
}