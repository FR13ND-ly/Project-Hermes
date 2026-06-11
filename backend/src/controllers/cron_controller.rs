use axum::{
    extract::{State, Path, Query},
    http::StatusCode,
    Json,
};
use uuid::Uuid;
use cron::Schedule;
use std::str::FromStr;
use chrono::Utc;

use crate::app_state::AppState;
use crate::models::cron_model::{CronJob, CronStatus, CronJobLog};
use crate::dtos::cron_dto::{CreateCronJobRequest, CronJobResponse, UpdateCronJobRequest, ProjectCronJobResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

pub async fn create_cron_job(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateCronJobRequest>,
) -> Result<(StatusCode, Json<CronJobResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let normalized_schedule = if payload.schedule.split_whitespace().count() == 5 {
        format!("0 {}", payload.schedule)
    } else {
        payload.schedule.clone()
    };

    let schedule = Schedule::from_str(&normalized_schedule).map_err(|e| {
        AppError::Validation(format!("Invalid cron expression: {}", e))
    })?;

    let next_run = schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));

    let id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO cron_jobs (id, workspace_id, project_id, app_id, name, schedule, command, next_run_at, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::cron_status)",
        id, ws_id, payload.project_id, payload.app_id, payload.name, normalized_schedule, payload.command, next_run, CronStatus::Active as _
    )
    .execute(&state.pool)
    .await?;

    if let Ok(job) = sqlx::query_as::<_, crate::models::cron_model::CronJob>(
        "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at 
         FROM cron_jobs 
         WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::CronJobUpdated {
                workspace_id: ws_id,
                job,
            }
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(CronJobResponse {
            id,
            app_id: payload.app_id,
            name: payload.name,
            schedule: normalized_schedule,
            command: payload.command,
            status: CronStatus::Active,
        }),
    ))
}

pub async fn delete_cron_job(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(job_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let deleted = sqlx::query!("DELETE FROM cron_jobs WHERE id = $1 AND workspace_id = $2", job_id, ws_id)
        .execute(&state.pool)
        .await?;

    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound("Cron job not found.".to_string()));
    }

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::CronJobDeleted {
            workspace_id: ws_id,
            job_id,
        }
    );

    Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
pub struct LogsQuery {
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

pub async fn list_cron_job_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(job_id): Path<Uuid>,
    Query(query): Query<LogsQuery>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let job_exists = sqlx::query!("SELECT id FROM cron_jobs WHERE id = $1 AND workspace_id = $2", job_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if job_exists.is_none() {
        return Err(AppError::NotFound("Cron job not found.".to_string()));
    }

    let page = query.page.unwrap_or(1);
    let limit = query.limit.unwrap_or(10);
    let offset = (page - 1) * limit;

    let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cron_job_logs WHERE cron_job_id = $1")
        .bind(job_id)
        .fetch_one(&state.pool)
        .await? ;

    let logs = sqlx::query_as::<_, CronJobLog>(
        "SELECT id, cron_job_id, exit_code, output, started_at, finished_at 
         FROM cron_job_logs 
         WHERE cron_job_id = $1 
         ORDER BY started_at DESC 
         LIMIT $2 OFFSET $3"
    )
    .bind(job_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let pages = (total as f64 / limit as f64).ceil() as i64;

    Ok(Json(serde_json::json!({
        "logs": logs,
        "total": total,
        "page": page,
        "limit": limit,
        "pages": pages
    })))
}

pub async fn list_app_cron_jobs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<Json<Vec<CronJob>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let jobs = sqlx::query_as::<_, CronJob>(
        "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at 
         FROM cron_jobs 
         WHERE app_id = $1 AND workspace_id = $2
         ORDER BY created_at DESC"
    )
    .bind(app_id)
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(jobs))
}

pub async fn list_project_cron_jobs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<ProjectCronJobResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let jobs = sqlx::query_as::<_, CronJob>(
        "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at
         FROM cron_jobs
         WHERE project_id = $1 AND workspace_id = $2
         ORDER BY created_at DESC"
    )
    .bind(project_id)
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    let mut response: Vec<ProjectCronJobResponse> = jobs
        .into_iter()
        .map(|j| ProjectCronJobResponse {
            id: j.id,
            app_id: j.app_id,
            name: j.name,
            schedule: j.schedule,
            command: j.command,
            status: j.status,
            next_run_at: j.next_run_at,
            source: "user".to_string(),
            database_id: None,
        })
        .collect();

    // Surface automatic database backups as synthetic, read-only cron entries.
    let auto_backups = sqlx::query!(
        "SELECT id, name, last_backup_at
         FROM databases
         WHERE project_id = $1 AND workspace_id = $2 AND backup_enabled = true
         ORDER BY name ASC",
        project_id,
        ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    for db in auto_backups {
        let next_run = db
            .last_backup_at
            .map(|t| t + chrono::Duration::hours(24))
            .unwrap_or_else(Utc::now);

        response.push(ProjectCronJobResponse {
            id: db.id,
            app_id: Uuid::nil(),
            name: format!("Auto-backup · {}", db.name),
            schedule: "0 0 * * *".to_string(),
            command: "Backup automat zilnic al bazei de date".to_string(),
            status: CronStatus::Active,
            next_run_at: Some(next_run),
            source: "backup".to_string(),
            database_id: Some(db.id),
        });
    }

    Ok(Json(response))
}

pub async fn update_cron_job(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(job_id): Path<Uuid>,
    Json(payload): Json<UpdateCronJobRequest>,
) -> Result<Json<CronJobResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let mut current_job = sqlx::query_as::<_, CronJob>(
        "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at 
         FROM cron_jobs 
         WHERE id = $1 AND workspace_id = $2"
    )
    .bind(job_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Cron job not found.".to_string()))?;

    if let Some(name) = payload.name {
        current_job.name = name;
    }
    if let Some(app_id) = payload.app_id {
        current_job.app_id = app_id;
    }
    if let Some(status) = payload.status {
        current_job.status = status;
    }
    if let Some(command) = payload.command {
        current_job.command = command;
    }

    if let Some(schedule_str) = payload.schedule {
        let normalized = if schedule_str.split_whitespace().count() == 5 {
            format!("0 {}", schedule_str)
        } else {
            schedule_str
        };
        // Validate schedule
        let schedule = Schedule::from_str(&normalized).map_err(|e| {
            AppError::Validation(format!("Invalid cron expression: {}", e))
        })?;
        current_job.schedule = normalized;

        if current_job.status == CronStatus::Active {
            current_job.next_run_at = schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));
        } else {
            current_job.next_run_at = None;
        }
    } else {
        if current_job.status == CronStatus::Active {
            let schedule = Schedule::from_str(&current_job.schedule).map_err(|e| {
                AppError::Validation(format!("Invalid cron expression: {}", e))
            })?;
            current_job.next_run_at = schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));
        } else {
            current_job.next_run_at = None;
        }
    }

    sqlx::query!(
        "UPDATE cron_jobs 
         SET name = $1, app_id = $2, schedule = $3, command = $4, status = $5::cron_status, next_run_at = $6, updated_at = NOW()
         WHERE id = $7 AND workspace_id = $8",
        current_job.name,
        current_job.app_id,
        current_job.schedule,
        current_job.command,
        current_job.status as _,
        current_job.next_run_at,
        current_job.id,
        ws_id
    )
    .execute(&state.pool)
    .await?;

    if let Ok(job) = sqlx::query_as::<_, crate::models::cron_model::CronJob>(
        "SELECT id, workspace_id, project_id, app_id, name, schedule, command, status, next_run_at, created_at, updated_at 
         FROM cron_jobs 
         WHERE id = $1"
    )
    .bind(current_job.id)
    .fetch_one(&state.pool)
    .await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::CronJobUpdated {
                workspace_id: ws_id,
                job,
            }
        );
    }

    Ok(Json(CronJobResponse {
        id: current_job.id,
        app_id: current_job.app_id,
        name: current_job.name,
        schedule: current_job.schedule,
        command: current_job.command,
        status: current_job.status,
    }))
}