use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;
use chrono::{DateTime, Utc};

use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentResponse {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub app_instance_id: Uuid,
    pub instance_name: String,
    pub incident_type: String,
    pub message: String,
    pub resolved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_project_incidents(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<IncidentResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let project_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        project_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !project_exists {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let records = sqlx::query!(
        "SELECT il.id, il.workspace_id, il.app_instance_id, il.incident_type, il.message, il.resolved_at, il.created_at, ai.container_name as instance_name
         FROM app_incident_logs il
         JOIN app_instances ai ON il.app_instance_id = ai.id
         JOIN apps a ON ai.app_id = a.id
         WHERE a.project_id = $1 AND il.workspace_id = $2
         ORDER BY il.created_at DESC",
        project_id, ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let response = records
        .into_iter()
        .map(|r| IncidentResponse {
            id: r.id,
            workspace_id: r.workspace_id,
            app_instance_id: r.app_instance_id,
            instance_name: r.instance_name,
            incident_type: r.incident_type,
            message: r.message,
            resolved_at: r.resolved_at,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(response))
}

pub async fn resolve_incident(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(incident_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let rows_affected = sqlx::query!(
        "UPDATE app_incident_logs 
         SET resolved_at = now() 
         WHERE id = $1 AND workspace_id = $2 AND resolved_at IS NULL",
        incident_id, ws_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(AppError::NotFound("Active incident not found or already resolved.".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}
