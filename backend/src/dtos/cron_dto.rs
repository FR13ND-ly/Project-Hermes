use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::models::cron_model::CronStatus;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCronJobRequest {
    pub project_id: Uuid,
    /// 'app' | 'database' | 'storage'. Defaults to 'app' for backward compatibility.
    #[serde(default = "default_target_type")]
    pub target_type: String,
    /// The targeted resource id. Falls back to `app_id` when omitted.
    #[serde(default)]
    pub target_id: Option<Uuid>,
    /// Legacy field — still accepted; equivalent to target_type='app', target_id=app_id.
    #[serde(default)]
    pub app_id: Option<Uuid>,
    pub name: String,
    pub schedule: String,
    pub command: String,
}

fn default_target_type() -> String {
    "app".to_string()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobResponse {
    pub id: Uuid,
    pub app_id: Option<Uuid>,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub is_backup: bool,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: CronStatus,
}

/// Project cron list entry. `source` is "backup" for managed database-backup crons,
/// "user" otherwise. `target_name` is the resolved app/database/bucket name.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCronJobResponse {
    pub id: Uuid,
    pub app_id: Option<Uuid>,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub target_name: Option<String>,
    pub is_backup: bool,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: CronStatus,
    pub next_run_at: Option<DateTime<Utc>>,
    /// "user" for regular cron jobs, "backup" for automatic database backups.
    pub source: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCronJobRequest {
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub app_id: Option<Uuid>,
    pub status: Option<CronStatus>,
}
