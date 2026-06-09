use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResponse {
    pub id: Uuid,
    pub database_id: Uuid,
    pub filename: String,
    pub file_size_bytes: i64,
    pub status: String,
    pub created_at: DateTime<Utc>,
}
