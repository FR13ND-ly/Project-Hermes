use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Deserialize)]
pub struct CreateSshKeyRequest {
    pub name: String,
    pub host: String,
    pub private_key: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectSshKeyResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub host: String,
    pub public_key: String,
    pub created_at: DateTime<Utc>,
}
