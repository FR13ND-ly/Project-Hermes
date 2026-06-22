use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::env_variable_dto::{
    LinkProjectEnvRequest, ProjectEnvResponse, RenameProjectEnvRequest, RevealResponse,
    SetProjectEnvRequest,
};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::controllers::env_variable_controller::hot_reload_if_running;
use crate::utils::{crypto, error::AppError};

fn clean_env_key(raw: &str) -> Result<String, AppError> {
    let clean = raw.trim().to_uppercase().replace(' ', "_");
    if clean.is_empty() {
        return Err(AppError::Validation("Variable key cannot be empty.".to_string()));
    }
    // Must be a valid C identifier — otherwise Kubernetes silently drops the env
    // var when injecting via envFrom secretRef (invalid names never reach the pod).
    let first_ok = clean.chars().next().map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false);
    let rest_ok = clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !first_ok || !rest_ok {
        return Err(AppError::Validation(format!(
            "Cheie de mediu invalidă '{}': folosește doar litere, cifre și underscore, fără a începe cu o cifră.",
            clean
        )));
    }
    Ok(clean)
}

/// Verify a project belongs to the caller's active workspace; returns workspace_id.
async fn authorize_project(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    expected_ws: Uuid,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!("SELECT workspace_id FROM projects WHERE id = $1", project_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Project not found.".to_string()))?;
    if row.workspace_id != expected_ws {
        return Err(AppError::Permission(
            "Project does not belong to the active workspace.".to_string(),
        ));
    }
    Ok(row.workspace_id)
}

/// Resolve and authorize the project owning an instance; returns project_id.
async fn authorize_instance_project(
    pool: &sqlx::PgPool,
    instance_id: Uuid,
    expected_ws: Uuid,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        "SELECT a.workspace_id, a.project_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1",
        instance_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("App instance not found.".to_string()))?;
    if row.workspace_id != expected_ws {
        return Err(AppError::Permission(
            "Instance does not belong to the active workspace.".to_string(),
        ));
    }
    Ok(row.project_id)
}

/// Hot-reload every running instance that links a given project env var.
async fn reload_linked_instances(pool: &sqlx::PgPool, project_env_id: Uuid) {
    if let Ok(insts) = sqlx::query_scalar!(
        "SELECT app_instance_id FROM app_env_links WHERE project_env_id = $1",
        project_env_id
    )
    .fetch_all(pool)
    .await
    {
        for inst in insts {
            hot_reload_if_running(pool, inst);
        }
    }
}

/// GET /projects/:project_id/env — list the project's env pool.
pub async fn list_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<ProjectEnvResponse>>, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_project(&state.pool, project_id, ws).await?;

    let rows = sqlx::query!(
        "SELECT id, project_id, key, encrypted_value, nonce, is_secret, source
         FROM project_env_variables WHERE project_id = $1 ORDER BY key ASC",
        project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let value = if !r.is_secret {
                crypto::decrypt_env_value(&r.encrypted_value, &r.nonce).ok()
            } else {
                None
            };
            ProjectEnvResponse {
                id: r.id,
                project_id: r.project_id,
                key: r.key,
                value,
                is_secret: r.is_secret,
                source: r.source,
                linked: None,
            }
        })
        .collect();

    Ok(Json(list))
}

/// POST /projects/:project_id/env — create or update a manual project env var.
pub async fn set_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<SetProjectEnvRequest>,
) -> Result<(StatusCode, Json<ProjectEnvResponse>), AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_project(&state.pool, project_id, ws).await?;

    let key = clean_env_key(&payload.key)?;
    let is_secret = payload.is_secret.unwrap_or(true);
    let (enc, nonce) = crypto::encrypt_env_value(&payload.value)?;

    let rec = sqlx::query!(
        "INSERT INTO project_env_variables (id, workspace_id, project_id, key, encrypted_value, nonce, is_secret, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'manual')
         ON CONFLICT (project_id, key)
         DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, source = 'manual', updated_at = now()
         RETURNING id",
        Uuid::new_v4(),
        ws,
        project_id,
        key,
        enc,
        nonce,
        is_secret
    )
    .fetch_one(&state.pool)
    .await?;

    reload_linked_instances(&state.pool, rec.id).await;

    let value = if !is_secret { Some(payload.value) } else { None };
    Ok((
        StatusCode::OK,
        Json(ProjectEnvResponse {
            id: rec.id,
            project_id,
            key,
            value,
            is_secret,
            source: "manual".to_string(),
            linked: None,
        }),
    ))
}

/// DELETE /projects/:project_id/env/:id — remove a project env var (cascades the
/// links away, propagating to every app that opted into it).
pub async fn delete_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_project(&state.pool, project_id, ws).await?;

    // Capture linked instances before the cascade removes the links.
    let linked: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT app_instance_id FROM app_env_links WHERE project_env_id = $1",
        id
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();

    let deleted = sqlx::query!(
        "DELETE FROM project_env_variables WHERE id = $1 AND project_id = $2",
        id,
        project_id
    )
    .execute(&state.pool)
    .await?;

    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound("Project env var not found.".to_string()));
    }

    for inst in linked {
        hot_reload_if_running(&state.pool, inst);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// GET /instances/:instance_id/project-env — the project pool available to this
/// instance, each flagged with whether the instance already links it.
pub async fn list_instance_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(instance_id): Path<Uuid>,
) -> Result<Json<Vec<ProjectEnvResponse>>, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let project_id = authorize_instance_project(&state.pool, instance_id, ws).await?;

    let rows = sqlx::query!(
        "SELECT pev.id, pev.project_id, pev.key, pev.encrypted_value, pev.nonce, pev.is_secret, pev.source,
                (ael.app_instance_id IS NOT NULL) AS \"linked!\"
         FROM project_env_variables pev
         LEFT JOIN app_env_links ael
           ON ael.project_env_id = pev.id AND ael.app_instance_id = $1
         WHERE pev.project_id = $2
         ORDER BY pev.key ASC",
        instance_id,
        project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let value = if !r.is_secret {
                crypto::decrypt_env_value(&r.encrypted_value, &r.nonce).ok()
            } else {
                None
            };
            ProjectEnvResponse {
                id: r.id,
                project_id: r.project_id,
                key: r.key,
                value,
                is_secret: r.is_secret,
                source: r.source,
                linked: Some(r.linked),
            }
        })
        .collect();

    Ok(Json(list))
}

/// POST /instances/:instance_id/env-links — opt an instance into a project var.
pub async fn link_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(instance_id): Path<Uuid>,
    Json(payload): Json<LinkProjectEnvRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let project_id = authorize_instance_project(&state.pool, instance_id, ws).await?;

    let belongs = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE id = $1 AND project_id = $2)",
        payload.project_env_id,
        project_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if !belongs {
        return Err(AppError::Permission(
            "Project env var is not in this app's project.".to_string(),
        ));
    }

    // "Linking wins": an explicit link supersedes a LOCAL var with the same key on
    // this instance. We delete the conflicting local one so a key can't be both local
    // and linked (which silently shadowed the linked value). This lets users set a
    // placeholder locally up-front and have it auto-replaced when the pool var links.
    let key: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT key FROM project_env_variables WHERE id = $1",
    )
    .bind(payload.project_env_id)
    .fetch_optional(&state.pool)
    .await?;

    let mut replaced_local_key: Option<String> = None;
    if let Some(ref k) = key {
        let deleted = sqlx::query("DELETE FROM environment_variables WHERE app_instance_id = $1 AND key = $2")
            .bind(instance_id)
            .bind(k)
            .execute(&state.pool)
            .await?;
        if deleted.rows_affected() > 0 {
            replaced_local_key = Some(k.clone());
        }
    }

    sqlx::query!(
        "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
        instance_id,
        payload.project_env_id
    )
    .execute(&state.pool)
    .await?;

    hot_reload_if_running(&state.pool, instance_id);
    Ok(Json(serde_json::json!({ "replacedLocalKey": replaced_local_key })))
}

/// PATCH /projects/:project_id/env/:id — rename a project env var's key. Works
/// for any source (manual or resource-published); a renamed resource var keeps its
/// name across later republishes. Linked instances are hot-reloaded.
pub async fn rename_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<RenameProjectEnvRequest>,
) -> Result<Json<ProjectEnvResponse>, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_project(&state.pool, project_id, ws).await?;

    let key = clean_env_key(&payload.key)?;

    // Reject a collision with a different var on the same key.
    let collision = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE project_id = $1 AND key = $2 AND id <> $3)",
        project_id, key, id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if collision {
        return Err(AppError::Validation(format!(
            "A project variable named {key} already exists."
        )));
    }

    let rec = sqlx::query!(
        "UPDATE project_env_variables SET key = $1, updated_at = now()
         WHERE id = $2 AND project_id = $3
         RETURNING id, project_id, key, encrypted_value, nonce, is_secret, source",
        key,
        id,
        project_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Project env var not found.".to_string()))?;

    reload_linked_instances(&state.pool, rec.id).await;

    let value = if !rec.is_secret {
        crypto::decrypt_env_value(&rec.encrypted_value, &rec.nonce).ok()
    } else {
        None
    };
    Ok(Json(ProjectEnvResponse {
        id: rec.id,
        project_id: rec.project_id,
        key: rec.key,
        value,
        is_secret: rec.is_secret,
        source: rec.source,
        linked: None,
    }))
}

/// GET /projects/:project_id/env/:id/reveal — decrypt and return a single var's
/// value on explicit request (so the UI can reveal secrets one at a time).
pub async fn reveal_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<RevealResponse>, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_project(&state.pool, project_id, ws).await?;

    let rec = sqlx::query!(
        "SELECT encrypted_value, nonce FROM project_env_variables WHERE id = $1 AND project_id = $2",
        id,
        project_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Project env var not found.".to_string()))?;

    let value = crypto::decrypt_env_value(&rec.encrypted_value, &rec.nonce)
        .map_err(|_| AppError::Infrastructure("Failed to decrypt value.".to_string()))?;

    Ok(Json(RevealResponse { value }))
}

/// DELETE /instances/:instance_id/env-links/:project_env_id — opt back out.
pub async fn unlink_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((instance_id, project_env_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws = claims
        .current_workspace_id
        .ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance_project(&state.pool, instance_id, ws).await?;

    sqlx::query!(
        "DELETE FROM app_env_links WHERE app_instance_id = $1 AND project_env_id = $2",
        instance_id,
        project_env_id
    )
    .execute(&state.pool)
    .await?;

    hot_reload_if_running(&state.pool, instance_id);
    Ok(StatusCode::NO_CONTENT)
}
