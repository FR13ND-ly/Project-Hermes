use axum::{
    extract::{State, Path},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::webhook_dto::{CreateWebhookRequest, WebhookResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

pub async fn create_webhook(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to workspace
    let project_exists = sqlx::query!(
        "SELECT id FROM projects WHERE id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if project_exists.is_none() {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let webhook_id = Uuid::new_v4();
    let created_at = chrono::Utc::now();
    let name = payload.name.trim().to_string();
    let url = payload.url.trim().to_string();
    let webhook_type = payload.webhook_type.trim().to_string();

    if name.is_empty() || url.is_empty() || webhook_type.is_empty() {
        return Err(AppError::Validation("All fields are required.".to_string()));
    }

    sqlx::query!(
        "INSERT INTO project_webhooks (id, project_id, name, url, webhook_type, is_active, created_at)
         VALUES ($1, $2, $3, $4, $5, true, $6)",
        webhook_id, project_id, name, url, webhook_type, created_at
    )
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(WebhookResponse {
            id: webhook_id,
            project_id,
            name,
            url,
            webhook_type,
            is_active: true,
            created_at,
        }),
    ))
}

pub async fn list_project_webhooks(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<WebhookResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to workspace
    let project_exists = sqlx::query!(
        "SELECT id FROM projects WHERE id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if project_exists.is_none() {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let records = sqlx::query!(
        "SELECT id, project_id, name, url, webhook_type, is_active, created_at
         FROM project_webhooks
         WHERE project_id = $1
         ORDER BY created_at DESC",
        project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let response = records
        .into_iter()
        .map(|r| WebhookResponse {
            id: r.id,
            project_id: r.project_id,
            name: r.name,
            url: r.url,
            webhook_type: r.webhook_type,
            is_active: r.is_active,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(response))
}

pub async fn delete_webhook(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, webhook_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to workspace
    let project_exists = sqlx::query!(
        "SELECT id FROM projects WHERE id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if project_exists.is_none() {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let result = sqlx::query!(
        "DELETE FROM project_webhooks WHERE id = $1 AND project_id = $2",
        webhook_id, project_id
    )
    .execute(&state.pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound("Webhook not found for this project.".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}
