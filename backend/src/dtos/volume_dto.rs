use serde::Serialize;
use uuid::Uuid;

/// A PVC listed in the central Storage interface, including its owning app and
/// whether it was auto-created at build (name starts with `auto-`).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectVolumeResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub app_name: String,
    pub name: String,
    pub container_path: String,
    pub host_path: String,
    pub is_auto: bool,
}
