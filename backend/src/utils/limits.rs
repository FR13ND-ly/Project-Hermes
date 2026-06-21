use sqlx::PgPool;
use uuid::Uuid;
use crate::utils::error::AppError;

pub async fn check_workspace_memory_limit(
    pool: &PgPool,
    workspace_id: Uuid,
    requested_memory_mb: i64,
    exclude_resource_id: Option<Uuid>,
) -> Result<(), AppError> {
    // 1. Get workspace limits
    let ws_limits = sqlx::query!(
        "SELECT max_memory_mb FROM workspaces WHERE id = $1",
        workspace_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Workspace not found.".to_string()))?;

    let max_mem = ws_limits.max_memory_mb as i64;
    if max_mem <= 0 {
        return Ok(());
    }

    // 2. Sum app instances memory
    let app_mem = match exclude_resource_id {
        Some(exclude_id) => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(ai.memory_limit_mb * ai.replicas_max), 0)::bigint
                 FROM app_instances ai
                 JOIN apps a ON ai.app_id = a.id
                 WHERE a.workspace_id = $1 AND ai.id != $2 AND ai.status NOT IN ('stopped', 'failed')",
                workspace_id,
                exclude_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
        None => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(ai.memory_limit_mb * ai.replicas_max), 0)::bigint
                 FROM app_instances ai
                 JOIN apps a ON ai.app_id = a.id
                 WHERE a.workspace_id = $1 AND ai.status NOT IN ('stopped', 'failed')",
                workspace_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
    };

    // 3. Sum databases memory
    let db_mem = match exclude_resource_id {
        Some(exclude_id) => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(d.memory_limit_mb), 0)::bigint 
                 FROM databases d 
                 WHERE d.workspace_id = $1 AND d.id != $2 AND d.status NOT IN ('stopped', 'failed')",
                workspace_id,
                exclude_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
        None => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(d.memory_limit_mb), 0)::bigint 
                 FROM databases d 
                 WHERE d.workspace_id = $1 AND d.status NOT IN ('stopped', 'failed')",
                workspace_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
    };

    // 4. Sum serverless functions memory
    let sf_mem = match exclude_resource_id {
        Some(exclude_id) => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(f.memory_limit_mb), 0)::bigint 
                 FROM serverless_instances f
                 WHERE f.workspace_id = $1 AND f.id != $2 AND f.status NOT IN ('stopped', 'failed')",
                workspace_id,
                exclude_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
        None => {
            sqlx::query_scalar!(
                "SELECT COALESCE(SUM(f.memory_limit_mb), 0)::bigint 
                 FROM serverless_instances f
                 WHERE f.workspace_id = $1 AND f.status NOT IN ('stopped', 'failed')",
                workspace_id
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0)
        }
    };

    let total_used = app_mem + db_mem + sf_mem;

    if total_used + requested_memory_mb > max_mem {
        return Err(AppError::Validation(format!(
            "Memoria totală alocată în workspace ({used} MB din {max} MB) ar depăși limita maximă admisă dacă se adaugă această resursă ({requested} MB). Vă rugăm să măriți limita de memorie a workspace-ului sau să opriți alte resurse.",
            used = total_used,
            max = max_mem,
            requested = requested_memory_mb
        )));
    }

    Ok(())
}

/// Mirror of `check_workspace_memory_limit` for CPU (millicores). `max_cpu_millicores`
/// of 0 means unlimited (the default), so the check is a no-op until an admin sets a cap.
pub async fn check_workspace_cpu_limit(
    pool: &PgPool,
    workspace_id: Uuid,
    requested_cpu_millicores: i32,
    exclude_resource_id: Option<Uuid>,
) -> Result<(), AppError> {
    let max_cpu = sqlx::query_scalar!(
        "SELECT max_cpu_millicores FROM workspaces WHERE id = $1",
        workspace_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Workspace not found.".to_string()))?;

    if max_cpu <= 0 {
        return Ok(());
    }

    let exclude = exclude_resource_id.unwrap_or_else(Uuid::nil);
    // Sum cpu_limit across active apps + databases (excluding the one being updated).
    // cpu_limit is in millicores. Serverless has no cpu_limit column (Knative manages
    // CPU itself), so it's not counted here.
    let total_used = sqlx::query_scalar!(
        r#"SELECT (
            COALESCE((SELECT SUM(ai.cpu_limit * ai.replicas_max) FROM app_instances ai JOIN apps a ON ai.app_id = a.id
                      WHERE a.workspace_id = $1 AND ai.id != $2 AND ai.status NOT IN ('stopped','failed')), 0)
          + COALESCE((SELECT SUM(d.cpu_limit) FROM databases d
                      WHERE d.workspace_id = $1 AND d.id != $2 AND d.status NOT IN ('stopped','failed')), 0)
        )::bigint"#,
        workspace_id,
        exclude
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(0);

    if total_used + requested_cpu_millicores as i64 > max_cpu as i64 {
        return Err(AppError::Validation(format!(
            "CPU-ul total alocat în workspace ({used}m din {max}m) ar depăși limita maximă dacă se adaugă această resursă ({requested}m). Măriți limita de CPU a workspace-ului sau opriți alte resurse.",
            used = total_used,
            max = max_cpu,
            requested = requested_cpu_millicores
        )));
    }

    Ok(())
}
