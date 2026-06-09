use axum::{
    extract::{Path, State, Query},
    http::StatusCode,
    Json,
};
use uuid::Uuid;
use serde::Deserialize;
use chrono::Utc;

use crate::app_state::AppState;
use crate::models::env_variable_model::{EnvironmentVariable, EnvScope};
use crate::dtos::env_variable_dto::{SetEnvRequest, EnvResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{crypto, error::AppError};



#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListEnvParams {
    pub project_id: Option<Uuid>,
    pub app_instance_id: Option<Uuid>,
    pub scope: Option<EnvScope>,
}

pub async fn set_env_variable(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<SetEnvRequest>,
) -> Result<(StatusCode, Json<EnvResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let clean_key = payload.key.trim().to_uppercase().replace(' ', "_");
    if clean_key.is_empty() {
        return Err(AppError::Validation("Variable key cannot be empty.".to_string()));
    }

    let is_secret = payload.is_secret.unwrap_or(true);
    let scope = payload.scope.unwrap_or(EnvScope::All);
    let (encrypted_value, nonce) = crypto::encrypt_env_value(&payload.value)?;
    let record_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO environment_variables (id, workspace_id, project_id, app_instance_id, key, encrypted_value, nonce, scope, is_secret)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (workspace_id, project_id, app_instance_id, key, scope) 
         DO UPDATE SET encrypted_value = $6, nonce = $7, is_secret = $9, updated_at = now()",
        record_id, 
        ws_id, 
        payload.project_id, 
        payload.app_instance_id, 
        clean_key, 
        encrypted_value, 
        nonce, 
        scope.clone() as EnvScope,
        is_secret
    )
    .execute(&state.pool)
    .await?;

    // Find and hot-reload all running instances affected by this environment variable change
    let pool_clone = state.pool.clone();
    let payload_app_instance_id = payload.app_instance_id;
    let payload_project_id = payload.project_id;
    tokio::spawn(async move {
        let affected_instances = sqlx::query!(
            "SELECT ai.id, ai.container_name, a.workspace_id, ai.status::TEXT as \"status!\", ai.meta_data
             FROM app_instances ai 
             JOIN apps a ON ai.app_id = a.id
             WHERE ai.status = 'running'
               AND ($1::uuid IS NULL OR ai.id = $1)
               AND ($2::uuid IS NULL OR a.project_id = $2)
               AND ($3::uuid IS NULL OR a.workspace_id = $3)",
            payload_app_instance_id,
            payload_project_id,
            Some(ws_id)
        )
        .fetch_all(&pool_clone)
        .await;

        if let Ok(instances) = affected_instances {
            for inst in instances {
                let _ = hot_reload_instance_envs(
                    &pool_clone,
                    inst.id,
                    &inst.container_name,
                    inst.workspace_id,
                    &inst.meta_data,
                )
                .await;
            }
        }
    });

    Ok((
        StatusCode::OK,
        Json(EnvResponse {
            id: record_id,
            project_id: payload.project_id,
            app_instance_id: payload.app_instance_id,
            key: clean_key,
            value: if is_secret { None } else { Some(payload.value) },
            scope,
            is_secret,
        }),
    ))
}


pub async fn list_env_variables(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(params): Query<ListEnvParams>,
) -> Result<Json<Vec<EnvResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let envs = if let Some(inst_id) = params.app_instance_id {
        // Resolve parent project_id
        let parent_project_id = sqlx::query!(
            "SELECT a.project_id FROM app_instances ai 
             JOIN apps a ON ai.app_id = a.id 
             WHERE ai.id = $1", 
            inst_id
        )
        .fetch_optional(&state.pool)
        .await?
        .map(|r| r.project_id);

        sqlx::query_as::<_, EnvironmentVariable>(
            "SELECT * FROM environment_variables 
             WHERE workspace_id = $1 
               AND (app_instance_id = $2 OR (app_instance_id IS NULL AND project_id = $3))
               AND (scope = $4 OR $4 IS NULL)
             ORDER BY key ASC"
        )
        .bind(ws_id)
        .bind(inst_id)
        .bind(parent_project_id)
        .bind(params.scope)
        .fetch_all(&state.pool)
        .await?
    } else if let Some(proj_id) = params.project_id {
        sqlx::query_as::<_, EnvironmentVariable>(
            "SELECT * FROM environment_variables 
             WHERE workspace_id = $1 
               AND project_id = $2 AND app_instance_id IS NULL
               AND (scope = $3 OR $3 IS NULL)
             ORDER BY key ASC"
        )
        .bind(ws_id)
        .bind(proj_id)
        .bind(params.scope)
        .fetch_all(&state.pool)
        .await?
    } else {
        sqlx::query_as::<_, EnvironmentVariable>(
            "SELECT * FROM environment_variables 
             WHERE workspace_id = $1 
               AND project_id IS NULL AND app_instance_id IS NULL
               AND (scope = $2 OR $2 IS NULL)
             ORDER BY key ASC"
        )
        .bind(ws_id)
        .bind(params.scope)
        .fetch_all(&state.pool)
        .await?
    };

    let response = envs
        .into_iter()
        .map(|env| {
            let decrypted_value = if !env.is_secret {
                crypto::decrypt_env_value(&env.encrypted_value, &env.nonce).ok()
            } else {
                None
            };

            EnvResponse {
                id: env.id,
                project_id: env.project_id,
                app_instance_id: env.app_instance_id,
                key: env.key,
                value: decrypted_value,
                scope: env.scope,
                is_secret: env.is_secret,
            }
        })
        .collect();

    Ok(Json(response))
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
         RETURNING project_id, app_instance_id",
        env_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if affected.is_none() {
        return Err(AppError::NotFound("Environment variable not found.".to_string()));
    }

    let record = affected.unwrap();

    // Find and hot-reload all running instances affected by this environment variable deletion
    let pool_clone = state.pool.clone();
    let record_app_instance_id = record.app_instance_id;
    let record_project_id = record.project_id;
    tokio::spawn(async move {
        let affected_instances = sqlx::query!(
            "SELECT ai.id, ai.container_name, a.workspace_id, ai.status::TEXT as \"status!\", ai.meta_data
             FROM app_instances ai 
             JOIN apps a ON ai.app_id = a.id
             WHERE ai.status = 'running'
               AND ($1::uuid IS NULL OR ai.id = $1)
               AND ($2::uuid IS NULL OR a.project_id = $2)
               AND ($3::uuid IS NULL OR a.workspace_id = $3)",
            record_app_instance_id,
            record_project_id,
            Some(ws_id)
        )
        .fetch_all(&pool_clone)
        .await;

        if let Ok(instances) = affected_instances {
            for inst in instances {
                let _ = hot_reload_instance_envs(
                    &pool_clone,
                    inst.id,
                    &inst.container_name,
                    inst.workspace_id,
                    &inst.meta_data,
                )
                .await;
            }
        }
    });

    Ok(StatusCode::NO_CONTENT)
}

async fn hot_reload_instance_envs(
    pool: &sqlx::PgPool,
    instance_id: Uuid,
    container_name: &str,
    workspace_id: Uuid,
    meta_data: &serde_json::Value,
) -> Result<(), crate::utils::error::AppError> {
    let instance_info = sqlx::query!(
        "SELECT a.project_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1",
        instance_id
    )
    .fetch_one(pool)
    .await?;

    let env_records = sqlx::query!(
        "SELECT key, encrypted_value, nonce FROM environment_variables 
         WHERE workspace_id = $1 
           AND (project_id = $2 OR project_id IS NULL)
           AND (app_instance_id = $3 OR app_instance_id IS NULL)",
        workspace_id,
        instance_info.project_id,
        instance_id
    )
    .fetch_all(pool)
    .await?;

    let mut envs = Vec::new();
    for rec in env_records {
        if let Ok(decrypted_value) = crate::utils::crypto::decrypt_env_value(&rec.encrypted_value, &rec.nonce) {
            envs.push((rec.key, decrypted_value));
        }
    }

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