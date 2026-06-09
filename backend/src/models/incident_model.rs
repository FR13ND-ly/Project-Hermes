use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppIncidentLog {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub app_instance_id: Uuid,
    pub incident_type: String,
    pub message: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}