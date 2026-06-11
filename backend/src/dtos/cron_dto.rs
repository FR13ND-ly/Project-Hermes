use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use crate::models::cron_model::CronStatus;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCronJobRequest {
    pub project_id: Uuid,
    pub app_id: Uuid,
    pub name: String,
    pub schedule: String,
    pub command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJobResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: CronStatus,
}

/// Project cron list entry. Includes both real user cron jobs and synthetic
/// system entries (e.g. automatic database backups) so they show up together.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCronJobResponse {
    pub id: Uuid,
    pub app_id: Uuid,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: CronStatus,
    pub next_run_at: Option<DateTime<Utc>>,
    /// "user" for regular cron jobs, "backup" for automatic database backups.
    pub source: String,
    pub database_id: Option<Uuid>,
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