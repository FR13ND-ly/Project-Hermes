use axum::{
    extract::{State, Path},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::chaos_dto::{ChaosExperimentResponse, StartChaosRequest};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

/// Start a chaos experiment against an app instance. Requires the instance to be
/// Running; `duration_sec` is capped inside `chaos::start_experiment`.
pub async fn start_chaos(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<StartChaosRequest>,
) -> Result<(StatusCode, Json<ChaosExperimentResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let inst = sqlx::query!(
        r#"SELECT ai.app_id, ai.container_name, ai.replicas_min, ai.status::text AS "status!", a.workspace_id
           FROM app_instances ai JOIN apps a ON ai.app_id = a.id
           WHERE ai.id = $1 AND ai.app_id = $2 AND a.workspace_id = $3"#,
        instance_id, app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application instance not found.".to_string()))?;

    if inst.status != "running" {
        return Err(AppError::Validation(
            "The instance must be Running to start a chaos experiment.".to_string(),
        ));
    }

    let exp = crate::utils::chaos::start_experiment(
        &state.pool,
        inst.workspace_id,
        inst.app_id,
        instance_id,
        &inst.container_name,
        inst.replicas_min,
        claims.sub,
        &req,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(exp)))
}

/// List the instance's chaos experiments (active first via recency).
pub async fn list_chaos(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ChaosExperimentResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Authorize: instance belongs to this app + workspace.
    let ok = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_instances ai JOIN apps a ON ai.app_id = a.id
                       WHERE ai.id = $1 AND ai.app_id = $2 AND a.workspace_id = $3)",
        instance_id, app_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if !ok {
        return Err(AppError::NotFound("Application instance not found.".to_string()));
    }

    let rows = sqlx::query!(
        r#"SELECT id, kind, status, message,
                  params as "params: serde_json::Value",
                  original_replicas, started_at, revert_at, ended_at
           FROM chaos_experiments
           WHERE app_instance_id = $1
           ORDER BY started_at DESC
           LIMIT 15"#,
        instance_id
    )
    .fetch_all(&state.pool)
    .await?;

    let items = rows
        .into_iter()
        .map(|r| ChaosExperimentResponse {
            id: r.id,
            kind: r.kind,
            status: r.status,
            message: r.message,
            params: r.params,
            original_replicas: r.original_replicas,
            started_at: r.started_at,
            revert_at: r.revert_at,
            ended_at: r.ended_at,
        })
        .collect();

    Ok(Json(items))
}

/// Stop a running experiment now (restore state immediately).
pub async fn cancel_chaos(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, instance_id, exp_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let running = sqlx::query_scalar!(
        "SELECT EXISTS(
            SELECT 1 FROM chaos_experiments ce
            JOIN app_instances ai ON ce.app_instance_id = ai.id
            JOIN apps a ON ai.app_id = a.id
            WHERE ce.id = $1 AND ce.app_instance_id = $2 AND ai.app_id = $3
              AND a.workspace_id = $4 AND ce.status = 'running')",
        exp_id, instance_id, app_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !running {
        return Err(AppError::NotFound("No running experiment with that id.".to_string()));
    }

    crate::utils::chaos::revert_experiment(&state.pool, exp_id, "cancelled").await;
    Ok(StatusCode::NO_CONTENT)
}
