use serde::Serialize;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub app_instance_id: Uuid,
    pub branch_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub commit_message: Option<String>,
    pub commit_sha: Option<String>,
    pub duration_sec: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDetailResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub app_instance_id: Uuid,
    pub branch_name: String,
    pub status: String,
    pub logs: String,
    pub created_at: DateTime<Utc>,
    pub commit_message: Option<String>,
    pub commit_sha: Option<String>,
    pub duration_sec: Option<i32>,
}
