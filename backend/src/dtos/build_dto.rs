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
    pub phase: String,
    pub failure_reason: Option<String>,
    pub failure_category: Option<String>,
    pub created_at: DateTime<Utc>,
    pub commit_message: Option<String>,
    pub commit_sha: Option<String>,
    pub duration_sec: Option<i32>,
    /// The immutable image this build produced (if any).
    pub image_tag: Option<String>,
    /// True when this build's image is the one currently deployed on the instance.
    pub is_live: bool,
}

/// An active piece of work across the workspace: an app build, a database
/// provisioning, or a serverless build. Used by the floating build indicator.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildQueueItem {
    /// Unique row id (build id for apps; resource id for db/serverless).
    pub id: Uuid,
    /// 'app' | 'database' | 'serverless'.
    pub kind: String,
    /// The resource id used for linking (app id / database id / function id).
    pub resource_id: Uuid,
    pub name: String,
    /// Branch (app) / type (db) / route (serverless).
    pub detail: Option<String>,
    pub project_id: Uuid,
    pub project_name: String,
    pub workspace_id: Uuid,
    pub workspace_name: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildDetailResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub app_instance_id: Uuid,
    pub branch_name: String,
    pub status: String,
    pub phase: String,
    pub failure_reason: Option<String>,
    pub failure_category: Option<String>,
    pub logs: String,
    pub created_at: DateTime<Utc>,
    pub commit_message: Option<String>,
    pub commit_sha: Option<String>,
    pub duration_sec: Option<i32>,
}
