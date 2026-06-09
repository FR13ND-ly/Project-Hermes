use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AppVolume {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub app_id: Uuid,
    pub name: String,
    pub container_path: String,
    pub host_path: String,
    pub created_at: DateTime<Utc>,
}