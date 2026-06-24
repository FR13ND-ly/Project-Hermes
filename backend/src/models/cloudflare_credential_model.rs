use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// A workspace-level Cloudflare credential — one token bundled with the single zone
/// (domain) it manages. Mirrors `GitCredential`. The token is stored encrypted; a
/// project references one via `projects.cloudflare_credential_id`.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct CloudflareCredential {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub label: String,
    pub encrypted_token: String,
    pub nonce: String,
    pub zone_id: String,
    pub base_domain: Option<String>,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}
