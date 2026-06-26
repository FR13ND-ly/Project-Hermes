use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;
use crate::utils::error::AppError;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct AuthAuditLog {
    pub id: Uuid,
    pub user_id: Option<Uuid>,
    pub identity: String,
    pub action: String,
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl AuthAuditLog {
    pub async fn record(
        pool: &PgPool,
        user_id: Option<Uuid>,
        identity: &str,
        action: &str,
        client_ip: Option<String>,
        user_agent: Option<String>,
    ) -> Result<(), AppError> {
        sqlx::query!(
            r#"
            INSERT INTO auth_audit_logs (user_id, identity, action, client_ip, user_agent)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            user_id,
            identity,
            action,
            client_ip,
            user_agent
        )
        .execute(pool)
        .await?;

        Ok(())
    }
}
