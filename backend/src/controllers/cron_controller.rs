use axum::{
    extract::{State, Path, Query},
    http::StatusCode,
    Json,
};
use std::collections::HashMap;
use uuid::Uuid;
use cron::Schedule;
use std::str::FromStr;
use chrono::Utc;

use crate::app_state::AppState;
use crate::models::cron_model::{CronJob, CronStatus, CronJobLog};
use crate::models::database_model::DbType;
use crate::dtos::cron_dto::{CreateCronJobRequest, CronJobResponse, UpdateCronJobRequest, ProjectCronJobResponse, CronEnvResponse, CronEnvVar};
use crate::dtos::env_variable_dto::EnvVarInput;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;
use crate::utils::crypto;
use crate::utils::pagination::{PaginationParams, Paginated};

/// Full column list for `cron_jobs` selects (keeps query_as in sync with the model).
const CRON_COLS: &str = "id, workspace_id, project_id, app_id, target_type, target_id, is_backup, name, schedule, command, status, next_run_at, created_at, updated_at";

/// Replace a cron's custom env vars wholesale (delete-then-insert). Keys are
/// normalized like app creation (uppercase, spaces→`_`); empty keys are skipped.
async fn replace_cron_custom_env(
    pool: &sqlx::PgPool,
    ws_id: Uuid,
    job_id: Uuid,
    vars: &[EnvVarInput],
) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM cron_env_variables WHERE cron_job_id = $1", job_id)
        .execute(pool)
        .await?;
    for var in vars {
        let key = var.key.trim().to_uppercase().replace(' ', "_");
        if key.is_empty() {
            continue;
        }
        let is_secret = var.is_secret.unwrap_or(true);
        let (enc, nonce) = crypto::encrypt_env_value(&var.value)?;
        sqlx::query!(
            "INSERT INTO cron_env_variables (id, workspace_id, cron_job_id, key, encrypted_value, nonce, is_secret)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (cron_job_id, key)
             DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, updated_at = now()",
            Uuid::new_v4(), ws_id, job_id, key, enc, nonce, is_secret
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Replace a cron's project-pool links wholesale. Each id is validated to belong to
/// the cron's project before linking.
async fn replace_cron_links(
    pool: &sqlx::PgPool,
    job_id: Uuid,
    project_id: Uuid,
    ids: &[Uuid],
) -> Result<(), AppError> {
    sqlx::query!("DELETE FROM cron_env_links WHERE cron_job_id = $1", job_id)
        .execute(pool)
        .await?;
    for id in ids {
        let belongs = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE id = $1 AND project_id = $2)",
            id, project_id
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);
        if !belongs {
            continue;
        }
        sqlx::query!(
            "INSERT INTO cron_env_links (cron_job_id, project_env_id) VALUES ($1, $2)
             ON CONFLICT DO NOTHING",
            job_id, id
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

fn normalize_schedule(raw: &str) -> String {
    if raw.split_whitespace().count() == 5 {
        format!("0 {}", raw)
    } else {
        raw.to_string()
    }
}

/// The default backup command for a database type. Runs inside the DB pod (so it
/// reads the pod's own credential env vars), stdout piped to the managed backup file.
pub fn default_backup_command(db_type: &DbType) -> String {
    // The dump writes to stdout (captured to the managed backup file). A trailing
    // `&& echo ... >&2` adds a friendly line to the history WITHOUT corrupting the
    // dump (stderr ≠ stdout) and preserves the dump's exit code (&& short-circuits).
    match db_type {
        DbType::Postgres => "pg_dump --clean --if-exists -U \"$POSTGRES_USER\" \"$POSTGRES_DB\" && echo \"✅ Postgres backup of database $POSTGRES_DB completed successfully.\" >&2".to_string(),
        DbType::Mysql => "mysqldump --add-drop-table -u\"$MYSQL_USER\" -p\"$MYSQL_PASSWORD\" \"$MYSQL_DATABASE\" && echo \"✅ MySQL backup of database $MYSQL_DATABASE completed successfully.\" >&2".to_string(),
        DbType::Mongodb => "mongodump --username \"$MONGO_INITDB_ROOT_USERNAME\" --password \"$MONGO_INITDB_ROOT_PASSWORD\" --authenticationDatabase admin --archive && echo \"✅ MongoDB backup completed successfully.\" >&2".to_string(),
        DbType::Redis => "redis-cli --rdb /dev/stdout && echo \"✅ Redis backup completed successfully.\" >&2".to_string(),
    }
}

/// Ensure a managed-backup cron exists for a database (idempotent). Called when
/// auto-backup is enabled and on startup reconciliation.
pub async fn ensure_backup_cron(pool: &sqlx::PgPool, db_id: Uuid) -> Result<(), AppError> {
    let db = sqlx::query!(
        "SELECT workspace_id, project_id, name, type as \"db_type: DbType\" FROM databases WHERE id = $1",
        db_id
    )
    .fetch_optional(pool)
    .await?;
    let Some(db) = db else { return Ok(()); };

    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE target_type = 'database' AND target_id = $1 AND is_backup = true)",
        db_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if exists {
        return Ok(());
    }

    let schedule = "0 0 3 * * *"; // daily at 03:00 (6-field with seconds)
    let next_run = Schedule::from_str(schedule)
        .ok()
        .and_then(|s| s.upcoming(Utc).next())
        .map(|dt| dt.with_timezone(&Utc));

    sqlx::query!(
        "INSERT INTO cron_jobs (id, workspace_id, project_id, app_id, target_type, target_id, is_backup, name, schedule, command, next_run_at, status)
         VALUES ($1, $2, $3, NULL, 'database', $4, true, $5, $6, $7, $8, 'active'::cron_status)",
        Uuid::new_v4(), db.workspace_id, db.project_id, db_id,
        format!("Backup · {}", db.name), schedule, default_backup_command(&db.db_type), next_run
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Remove the managed-backup cron for a database (when auto-backup is disabled).
pub async fn remove_backup_cron(pool: &sqlx::PgPool, db_id: Uuid) -> Result<(), AppError> {
    sqlx::query!(
        "DELETE FROM cron_jobs WHERE target_type = 'database' AND target_id = $1 AND is_backup = true",
        db_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn create_cron_job(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateCronJobRequest>,
) -> Result<(StatusCode, Json<CronJobResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let target_type = payload.target_type.to_lowercase();
    let target_id = payload.target_id.or(payload.app_id).ok_or_else(|| {
        AppError::Validation("A target resource (app, database or storage) is required.".to_string())
    })?;

    // Validate the target belongs to this workspace + project.
    let valid = match target_type.as_str() {
        "app" => sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM apps WHERE id = $1 AND workspace_id = $2 AND project_id = $3)",
            target_id, ws_id, payload.project_id
        ).fetch_one(&state.pool).await?.unwrap_or(false),
        "database" => sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM databases WHERE id = $1 AND workspace_id = $2 AND project_id = $3)",
            target_id, ws_id, payload.project_id
        ).fetch_one(&state.pool).await?.unwrap_or(false),
        "storage" => sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM storage_buckets WHERE id = $1 AND workspace_id = $2 AND project_id = $3)",
            target_id, ws_id, payload.project_id
        ).fetch_one(&state.pool).await?.unwrap_or(false),
        other => return Err(AppError::Validation(format!("Unknown target type '{}'.", other))),
    };
    if !valid {
        return Err(AppError::NotFound("Target resource not found in this project.".to_string()));
    }

    let normalized_schedule = normalize_schedule(&payload.schedule);
    let schedule = Schedule::from_str(&normalized_schedule)
        .map_err(|e| AppError::Validation(format!("Invalid cron expression: {}", e)))?;
    let next_run = schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));

    let app_id_col = if target_type == "app" { Some(target_id) } else { None };
    let id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO cron_jobs (id, workspace_id, project_id, app_id, target_type, target_id, is_backup, name, schedule, command, next_run_at, status)
         VALUES ($1, $2, $3, $4, $5, $6, false, $7, $8, $9, $10, 'active'::cron_status)",
        id, ws_id, payload.project_id, app_id_col, target_type, target_id, payload.name, normalized_schedule, payload.command, next_run
    )
    .execute(&state.pool)
    .await?;

    // Persist the cron's env: custom vars + project-pool links (mirrors app creation).
    if let Some(vars) = &payload.env_variables {
        replace_cron_custom_env(&state.pool, ws_id, id, vars).await?;
    }
    if let Some(ids) = &payload.linked_project_env_ids {
        replace_cron_links(&state.pool, id, payload.project_id, ids).await?;
    }

    if let Ok(job) = sqlx::query_as::<_, CronJob>(&format!("SELECT {CRON_COLS} FROM cron_jobs WHERE id = $1"))
        .bind(id)
        .fetch_one(&state.pool)
        .await
    {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::CronJobUpdated { workspace_id: ws_id, job }
        );
    }

    Ok((
        StatusCode::CREATED,
        Json(CronJobResponse {
            id,
            app_id: app_id_col,
            target_type,
            target_id: Some(target_id),
            is_backup: false,
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

    // If this is a managed backup cron, also turn off the database's backup flag so
    // it isn't recreated by reconciliation.
    if let Some(row) = sqlx::query!(
        "SELECT target_id, is_backup FROM cron_jobs WHERE id = $1 AND workspace_id = $2",
        job_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    {
        if row.is_backup {
            if let Some(db_id) = row.target_id {
                let _ = sqlx::query!("UPDATE databases SET backup_enabled = false WHERE id = $1", db_id)
                    .execute(&state.pool).await;
            }
        }
    }

    let deleted = sqlx::query!("DELETE FROM cron_jobs WHERE id = $1 AND workspace_id = $2", job_id, ws_id)
        .execute(&state.pool)
        .await?;

    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound("Cron job not found.".to_string()));
    }

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::CronJobDeleted { workspace_id: ws_id, job_id }
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
        .await?;

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
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<CronJob>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM cron_jobs WHERE target_type = 'app' AND target_id = $1 AND workspace_id = $2",
        app_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let jobs = sqlx::query_as::<_, CronJob>(&format!(
        "SELECT {CRON_COLS} FROM cron_jobs
         WHERE target_type = 'app' AND target_id = $1 AND workspace_id = $2
         ORDER BY created_at DESC
         LIMIT $3 OFFSET $4"
    ))
    .bind(app_id)
    .bind(ws_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(Paginated::new(jobs, total, page, page_size)))
}

pub async fn list_project_cron_jobs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<ProjectCronJobResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM cron_jobs WHERE project_id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let jobs = sqlx::query_as::<_, CronJob>(&format!(
        "SELECT {CRON_COLS} FROM cron_jobs
         WHERE project_id = $1 AND workspace_id = $2
         ORDER BY is_backup ASC, created_at DESC
         LIMIT $3 OFFSET $4"
    ))
    .bind(project_id)
    .bind(ws_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    // Build name maps so we can resolve each cron's target name in one pass.
    let mut names: HashMap<Uuid, String> = HashMap::new();
    if let Ok(rows) = sqlx::query!("SELECT id, name FROM apps WHERE project_id = $1", project_id).fetch_all(&state.pool).await {
        for r in rows { names.insert(r.id, r.name); }
    }
    if let Ok(rows) = sqlx::query!("SELECT id, name FROM databases WHERE project_id = $1", project_id).fetch_all(&state.pool).await {
        for r in rows { names.insert(r.id, r.name); }
    }
    if let Ok(rows) = sqlx::query!("SELECT id, name FROM storage_buckets WHERE project_id = $1", project_id).fetch_all(&state.pool).await {
        for r in rows { names.insert(r.id, r.name); }
    }

    let items: Vec<ProjectCronJobResponse> = jobs
        .into_iter()
        .map(|j| {
            let target_name = j.target_id.and_then(|tid| names.get(&tid).cloned());
            let source = if j.is_backup { "backup" } else { "user" }.to_string();
            ProjectCronJobResponse {
                id: j.id,
                app_id: j.app_id,
                target_type: j.target_type,
                target_id: j.target_id,
                target_name,
                is_backup: j.is_backup,
                name: j.name,
                schedule: j.schedule,
                command: j.command,
                status: j.status,
                next_run_at: j.next_run_at,
                source,
            }
        })
        .collect();

    Ok(Json(Paginated::new(items, total, page, page_size)))
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

    let mut current_job = sqlx::query_as::<_, CronJob>(&format!(
        "SELECT {CRON_COLS} FROM cron_jobs WHERE id = $1 AND workspace_id = $2"
    ))
    .bind(job_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Cron job not found.".to_string()))?;

    if let Some(name) = payload.name {
        current_job.name = name;
    }
    // app_id is only swappable for app-targeted crons.
    if let Some(app_id) = payload.app_id {
        if current_job.target_type == "app" {
            current_job.app_id = Some(app_id);
            current_job.target_id = Some(app_id);
        }
    }
    if let Some(status) = payload.status {
        current_job.status = status;
    }
    if let Some(command) = payload.command {
        current_job.command = command;
    }

    if let Some(schedule_str) = payload.schedule {
        let normalized = normalize_schedule(&schedule_str);
        let schedule = Schedule::from_str(&normalized)
            .map_err(|e| AppError::Validation(format!("Invalid cron expression: {}", e)))?;
        current_job.schedule = normalized;
        current_job.next_run_at = if current_job.status == CronStatus::Active {
            schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc))
        } else {
            None
        };
    } else if current_job.status == CronStatus::Active {
        let schedule = Schedule::from_str(&current_job.schedule)
            .map_err(|e| AppError::Validation(format!("Invalid cron expression: {}", e)))?;
        current_job.next_run_at = schedule.upcoming(Utc).next().map(|dt| dt.with_timezone(&Utc));
    } else {
        current_job.next_run_at = None;
    }

    sqlx::query!(
        "UPDATE cron_jobs
         SET name = $1, app_id = $2, target_id = $3, schedule = $4, command = $5, status = $6::cron_status, next_run_at = $7, updated_at = NOW()
         WHERE id = $8 AND workspace_id = $9",
        current_job.name,
        current_job.app_id,
        current_job.target_id,
        current_job.schedule,
        current_job.command,
        current_job.status as _,
        current_job.next_run_at,
        current_job.id,
        ws_id
    )
    .execute(&state.pool)
    .await?;

    // Env edits are replace-all: only touched when the field is present in the payload.
    if let Some(vars) = &payload.env_variables {
        replace_cron_custom_env(&state.pool, ws_id, current_job.id, vars).await?;
    }
    if let Some(ids) = &payload.linked_project_env_ids {
        replace_cron_links(&state.pool, current_job.id, current_job.project_id, ids).await?;
    }

    if let Ok(job) = sqlx::query_as::<_, CronJob>(&format!("SELECT {CRON_COLS} FROM cron_jobs WHERE id = $1"))
        .bind(current_job.id)
        .fetch_one(&state.pool)
        .await
    {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::CronJobUpdated { workspace_id: ws_id, job }
        );
    }

    Ok(Json(CronJobResponse {
        id: current_job.id,
        app_id: current_job.app_id,
        target_type: current_job.target_type,
        target_id: current_job.target_id,
        is_backup: current_job.is_backup,
        name: current_job.name,
        schedule: current_job.schedule,
        command: current_job.command,
        status: current_job.status,
    }))
}

/// GET /cron/:job_id/env — the cron's configured env: its custom vars (value
/// omitted for secrets) + the ids of the project-pool vars it links. Used to
/// prefill the edit form.
pub async fn get_cron_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(job_id): Path<Uuid>,
) -> Result<Json<CronEnvResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id = $1 AND workspace_id = $2)",
        job_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if !exists {
        return Err(AppError::NotFound("Cron job not found.".to_string()));
    }

    let rows = sqlx::query!(
        "SELECT id, key, encrypted_value, nonce, is_secret FROM cron_env_variables WHERE cron_job_id = $1 ORDER BY key ASC",
        job_id
    )
    .fetch_all(&state.pool)
    .await?;

    let variables = rows
        .into_iter()
        .map(|r| {
            let value = if !r.is_secret {
                crypto::decrypt_env_value(&r.encrypted_value, &r.nonce).ok()
            } else {
                None
            };
            CronEnvVar { id: r.id, key: r.key, value, is_secret: r.is_secret }
        })
        .collect();

    let linked_project_env_ids = sqlx::query_scalar!(
        "SELECT project_env_id FROM cron_env_links WHERE cron_job_id = $1",
        job_id
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(CronEnvResponse { variables, linked_project_env_ids }))
}
