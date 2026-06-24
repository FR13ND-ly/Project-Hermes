use axum::{extract::{State, Path}, Json};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::cloudflare_credential_model::CloudflareCredential;
use crate::utils::{crypto, error::AppError};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCloudflareCredentialRequest {
    pub label: String,
    pub token: String,
    pub zone_id: String,
    pub base_domain: Option<String>,
}

/// Public view — never exposes the token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudflareCredentialResponse {
    pub id: Uuid,
    pub label: String,
    pub zone_id: String,
    pub base_domain: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl From<CloudflareCredential> for CloudflareCredentialResponse {
    fn from(c: CloudflareCredential) -> Self {
        CloudflareCredentialResponse {
            id: c.id,
            label: c.label,
            zone_id: c.zone_id,
            base_domain: c.base_domain,
            created_at: c.created_at,
        }
    }
}

/// GET /cloudflare-credentials — list the workspace's Cloudflare tokens (no secrets).
pub async fn list_cloudflare_credentials(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<CloudflareCredentialResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let creds = sqlx::query_as::<_, CloudflareCredential>(
        "SELECT * FROM cloudflare_credentials WHERE workspace_id = $1 ORDER BY created_at DESC"
    )
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(creds.into_iter().map(Into::into).collect()))
}

/// POST /cloudflare-credentials — create a credential (token encrypted at rest).
pub async fn create_cloudflare_credential(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateCloudflareCredentialRequest>,
) -> Result<Json<CloudflareCredentialResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    let label = payload.label.trim();
    if label.is_empty() {
        return Err(AppError::Validation("The credential label is required.".to_string()));
    }
    let token = payload.token.trim();
    if token.is_empty() {
        return Err(AppError::Validation("The token is required.".to_string()));
    }
    let zone_id = payload.zone_id.trim();
    if zone_id.is_empty() {
        return Err(AppError::Validation("The Zone ID is required.".to_string()));
    }
    let base_domain = payload.base_domain.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let (encrypted_token, nonce) = crypto::encrypt_env_value(token)?;
    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO cloudflare_credentials (id, workspace_id, label, encrypted_token, nonce, zone_id, base_domain, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        id, ws_id, label, encrypted_token, nonce, zone_id, base_domain, claims.sub
    )
    .execute(&state.pool)
    .await?;

    let c = sqlx::query_as::<_, CloudflareCredential>("SELECT * FROM cloudflare_credentials WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok(Json(c.into()))
}

/// DELETE /cloudflare-credentials/:id — remove a credential (projects detach via FK).
pub async fn delete_cloudflare_credential(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let deleted = sqlx::query!(
        "DELETE FROM cloudflare_credentials WHERE id = $1 AND workspace_id = $2", id, ws_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();
    if deleted == 0 {
        return Err(AppError::NotFound("Cloudflare credential not found.".to_string()));
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
