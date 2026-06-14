use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    Json,
};
use uuid::Uuid;
use serde::Deserialize;
use chrono::Utc;

use crate::app_state::AppState;
use crate::models::env_variable_model::EnvironmentVariable;
use crate::dtos::env_variable_dto::{
    SetEnvRequest, SetEnvBulkRequest, EnvResponse, GroupedAppEnv, GroupedInstanceEnv,
};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{crypto, error::AppError};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvParams {
    pub app_instance_id: Uuid,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// Resolve and authorize the workspace that owns a given app instance.
/// Returns the workspace_id when the instance belongs to the caller's active workspace.
async fn resolve_instance_workspace(
    pool: &sqlx::PgPool,
    instance_id: Uuid,
    expected_ws: Uuid,
) -> Result<Uuid, AppError> {
    let row = sqlx::query!(
        "SELECT a.workspace_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1",
        instance_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("App instance not found.".to_string()))?;

    if row.workspace_id != expected_ws {
        return Err(AppError::Permission("Instance does not belong to the active workspace.".to_string()));
    }

    Ok(row.workspace_id)
}

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

pub async fn set_env_variable(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<SetEnvRequest>,
) -> Result<(StatusCode, Json<EnvResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let workspace_id = resolve_instance_workspace(&state.pool, payload.app_instance_id, ws_id).await?;

    let clean_key = clean_env_key(&payload.key)?;
    let is_secret = payload.is_secret.unwrap_or(true);
    let (encrypted_value, nonce) = crypto::encrypt_env_value(&payload.value)?;
    let record_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (app_instance_id, key)
         DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, updated_at = now()",
        record_id,
        workspace_id,
        payload.app_instance_id,
        clean_key,
        encrypted_value,
        nonce,
        is_secret
    )
    .execute(&state.pool)
    .await?;

    hot_reload_if_running(&state.pool, payload.app_instance_id);

    Ok((
        StatusCode::OK,
        Json(EnvResponse {
            id: record_id,
            app_instance_id: payload.app_instance_id,
            key: clean_key,
            value: if is_secret { None } else { Some(payload.value) },
            is_secret,
        }),
    ))
}

/// Replace the entire set of env vars for one instance in a single shot.
/// Powers the "edit .env as JSON" workflow.
pub async fn set_envs_bulk(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<SetEnvBulkRequest>,
) -> Result<(StatusCode, Json<Vec<EnvResponse>>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let workspace_id = resolve_instance_workspace(&state.pool, payload.app_instance_id, ws_id).await?;

    let mut tx = state.pool.begin().await?;

    sqlx::query!(
        "DELETE FROM environment_variables WHERE app_instance_id = $1",
        payload.app_instance_id
    )
    .execute(&mut *tx)
    .await?;

    let mut response = Vec::new();
    for var in &payload.variables {
        let clean_key = clean_env_key(&var.key)?;
        let is_secret = var.is_secret.unwrap_or(true);
        let (encrypted_value, nonce) = crypto::encrypt_env_value(&var.value)?;
        let record_id = Uuid::new_v4();

        sqlx::query!(
            "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (app_instance_id, key)
             DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, updated_at = now()",
            record_id,
            workspace_id,
            payload.app_instance_id,
            clean_key,
            encrypted_value,
            nonce,
            is_secret
        )
        .execute(&mut *tx)
        .await?;

        response.push(EnvResponse {
            id: record_id,
            app_instance_id: payload.app_instance_id,
            key: clean_key,
            value: if is_secret { None } else { Some(var.value.clone()) },
            is_secret,
        });
    }

    tx.commit().await?;

    hot_reload_if_running(&state.pool, payload.app_instance_id);

    Ok((StatusCode::OK, Json(response)))
}

pub async fn list_env_variables(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(params): Query<ListEnvParams>,
) -> Result<Json<crate::utils::pagination::Paginated<EnvResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    resolve_instance_workspace(&state.pool, params.app_instance_id, ws_id).await?;

    let (page, page_size, offset) = crate::utils::pagination::PaginationParams {
        page: params.page,
        page_size: params.page_size,
    }.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM environment_variables WHERE app_instance_id = $1",
        params.app_instance_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let envs = sqlx::query_as::<_, EnvironmentVariable>(
        "SELECT * FROM environment_variables
         WHERE app_instance_id = $1
         ORDER BY key ASC
         LIMIT $2 OFFSET $3"
    )
    .bind(params.app_instance_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let items = envs.into_iter().map(to_env_response).collect();
    Ok(Json(crate::utils::pagination::Paginated::new(items, total, page, page_size)))
}

/// Project-level view: every app in the project, with its instances and their env.
pub async fn list_project_envs_grouped(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<GroupedAppEnv>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let apps = sqlx::query!(
        "SELECT id, name FROM apps
         WHERE project_id = $1 AND workspace_id = $2
         ORDER BY name ASC",
        project_id,
        ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let mut grouped = Vec::new();
    for app in apps {
        let instances = sqlx::query!(
            "SELECT id, branch_name FROM app_instances
             WHERE app_id = $1
             ORDER BY branch_name ASC",
            app.id
        )
        .fetch_all(&state.pool)
        .await?;

        let mut instance_groups = Vec::new();
        for inst in instances {
            let envs = sqlx::query_as::<_, EnvironmentVariable>(
                "SELECT * FROM environment_variables
                 WHERE app_instance_id = $1
                 ORDER BY key ASC"
            )
            .bind(inst.id)
            .fetch_all(&state.pool)
            .await?;

            instance_groups.push(GroupedInstanceEnv {
                instance_id: inst.id,
                branch_name: inst.branch_name,
                variables: envs.into_iter().map(to_env_response).collect(),
            });
        }

        grouped.push(GroupedAppEnv {
            app_id: app.id,
            app_name: app.name,
            instances: instance_groups,
        });
    }

    Ok(Json(grouped))
}

pub async fn delete_env_variable(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(env_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let affected = sqlx::query!(
        "DELETE FROM environment_variables
         WHERE id = $1 AND workspace_id = $2
         RETURNING app_instance_id",
        env_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    let record = affected.ok_or_else(|| {
        AppError::NotFound("Environment variable not found.".to_string())
    })?;

    hot_reload_if_running(&state.pool, record.app_instance_id);

    Ok(StatusCode::NO_CONTENT)
}

fn to_env_response(env: EnvironmentVariable) -> EnvResponse {
    let decrypted_value = if !env.is_secret {
        crypto::decrypt_env_value(&env.encrypted_value, &env.nonce).ok()
    } else {
        None
    };

    EnvResponse {
        id: env.id,
        app_instance_id: env.app_instance_id,
        key: env.key,
        value: decrypted_value,
        is_secret: env.is_secret,
    }
}

/// Spawn a background hot-reload of the instance if it is currently running.
pub(crate) fn hot_reload_if_running(pool: &sqlx::PgPool, instance_id: Uuid) {
    let pool = pool.clone();
    tokio::spawn(async move {
        let instance = sqlx::query!(
            "SELECT ai.container_name, a.workspace_id, ai.status::TEXT as \"status!\", ai.meta_data
             FROM app_instances ai
             JOIN apps a ON ai.app_id = a.id
             WHERE ai.id = $1 AND ai.status = 'running'",
            instance_id
        )
        .fetch_optional(&pool)
        .await;

        if let Ok(Some(inst)) = instance {
            let _ = hot_reload_instance_envs(
                &pool,
                instance_id,
                &inst.container_name,
                inst.workspace_id,
                &inst.meta_data,
            )
            .await;
        }
    });
}

async fn hot_reload_instance_envs(
    pool: &sqlx::PgPool,
    instance_id: Uuid,
    container_name: &str,
    workspace_id: Uuid,
    meta_data: &serde_json::Value,
) -> Result<(), crate::utils::error::AppError> {
    // Effective env = linked project-pool vars + this instance's own vars.
    let envs = crate::utils::app_env::resolve_instance_env(pool, instance_id).await;

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", workspace_id);
    crate::utils::k8s::K8sManager::create_secret(&k8s_client, &namespace, container_name, envs).await?;

    let knative_enabled = meta_data.get("knative_enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let restart_at = Utc::now().to_rfc3339();

    if knative_enabled {
        let gvk = kube::api::GroupVersionKind::gvk("serving.knative.dev", "v1", "Service");
        let api_resource = kube::api::ApiResource::from_gvk_with_plural(&gvk, "services");
        let dynamic_api = kube::Api::<kube::core::DynamicObject>::namespaced_with(
            k8s_client,
            &namespace,
            &api_resource
        );

        let patch = serde_json::json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "hermes.io/env-updated-at": restart_at
                        }
                    }
                }
            }
        });

        let _ = dynamic_api.patch(
            container_name,
            &kube::api::PatchParams::default(),
            &kube::api::Patch::Merge(&patch),
        ).await.map_err(|e| crate::utils::error::AppError::Infrastructure(format!("Failed to patch Knative Service: {}", e)))?;
    } else {
        let deployments: kube::Api<k8s_openapi::api::apps::v1::Deployment> =
            kube::Api::namespaced(k8s_client, &namespace);

        let patch = serde_json::json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "kubectl.kubernetes.io/restartedAt": restart_at
                        }
                    }
                }
            }
        });

        let _ = deployments.patch(
            container_name,
            &kube::api::PatchParams::apply("hermes-orchestrator"),
            &kube::api::Patch::Merge(&patch),
        ).await.map_err(|e| crate::utils::error::AppError::Infrastructure(format!("Failed to patch Deployment: {}", e)))?;
    }

    Ok(())
}
