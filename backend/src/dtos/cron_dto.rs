use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::models::cron_model::CronStatus;
use crate::dtos::env_variable_dto::EnvVarInput;

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
    /// Custom env vars for the cron run (a key already in the project pool is linked
    /// instead of duplicated, mirroring app creation).
    #[serde(default)]
    pub env_variables: Option<Vec<EnvVarInput>>,
    /// Project-pool env vars to link this cron to.
    #[serde(default)]
    pub linked_project_env_ids: Option<Vec<Uuid>>,
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
    /// When present, replaces the cron's custom env vars wholesale (replace-all,
    /// like the instance bulk-env editor). Omit to leave env untouched.
    #[serde(default)]
    pub env_variables: Option<Vec<EnvVarInput>>,
    /// When present, replaces the cron's project-pool links wholesale.
    #[serde(default)]
    pub linked_project_env_ids: Option<Vec<Uuid>>,
}

/// A single custom cron env var (value omitted for secrets).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronEnvVar {
    pub id: Uuid,
    pub key: String,
    pub value: Option<String>,
    pub is_secret: bool,
}

/// The current env configured on a cron: its custom vars + the ids of the
/// project-pool vars it links (used to prefill the edit form).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronEnvResponse {
    pub variables: Vec<CronEnvVar>,
    pub linked_project_env_ids: Vec<Uuid>,
}
