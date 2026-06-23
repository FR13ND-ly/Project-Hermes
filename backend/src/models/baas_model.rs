use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};

/// A standalone BaaS (end-user auth) service — a first-class project resource,
/// independent of any app. It owns the end-user identity namespace and the
/// role→permission config; the signing secret itself lives in the project env pool
/// (source='baas_auth', source_id = this id) so apps can link it like any pool var.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct BaasService {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub slug: String,
    pub auth_roles_config: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
