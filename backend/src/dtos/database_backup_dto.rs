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
    /// Any message the backup command emitted on stderr (e.g. a friendly echo),
    /// surfaced into the cron history. Not persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log: Option<String>,
}
