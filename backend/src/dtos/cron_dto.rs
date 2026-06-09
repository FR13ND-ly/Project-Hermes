use serde::{Deserialize, Serialize};
use uuid::Uuid;
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCronJobRequest {
    pub name: Option<String>,
    pub schedule: Option<String>,
    pub command: Option<String>,
    pub app_id: Option<Uuid>,
    pub status: Option<CronStatus>,
}