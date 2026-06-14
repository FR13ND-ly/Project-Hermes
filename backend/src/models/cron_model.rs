use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize, PartialEq)]
#[sqlx(type_name = "cron_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum CronStatus {
    Active,
    Paused,
    Failed,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CronJob {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    /// Set only for app-targeted crons (kept for backward compatibility).
    pub app_id: Option<Uuid>,
    /// 'app' | 'database' | 'storage'.
    pub target_type: String,
    /// The targeted resource id (app / database / storage bucket).
    pub target_id: Option<Uuid>,
    /// True for the managed database-backup cron.
    pub is_backup: bool,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: CronStatus,
    pub next_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CronJobLog {
    pub id: Uuid,
    pub cron_job_id: Uuid,
    pub exit_code: i32,
    pub output: Option<String>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}