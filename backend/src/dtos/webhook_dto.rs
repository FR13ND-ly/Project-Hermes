use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateWebhookRequest {
    pub name: String,
    pub url: String,
    pub webhook_type: String, // 'slack', 'discord', 'generic'
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookResponse {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub url: String,
    pub webhook_type: String,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}
