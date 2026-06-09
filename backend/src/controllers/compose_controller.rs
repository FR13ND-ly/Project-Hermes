use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use uuid::Uuid;
use std::collections::HashMap;

use crate::app_state::AppState;
use crate::dtos::compose_dto::{ImportComposeRequest, ComposeStack, EnvironmentMapping};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

pub async fn import_compose_stack(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<ImportComposeRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let project_exists = sqlx::query!(
        "SELECT id FROM projects WHERE id = $1 AND workspace_id = $2",
        payload.project_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if project_exists.is_none() {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let stack: ComposeStack = serde_yaml::from_str(&payload.compose_yaml).map_err(|e| {
        AppError::Validation(format!("Invalid Docker Compose YAML format: {}", e))
    })?;

    let mut tx = state.pool.begin().await?;

    for (service_name, service_data) in stack.services {
        let app_id = Uuid::new_v4();
        let display_name = format!("{}-{}", service_name, &Uuid::new_v4().to_string()[..6]);
        let slug = display_name.to_lowercase().replace('_', "-");
        
        let is_addon_db = service_data.image.as_ref().map_or(false, |img| {
            let img_lower = img.to_lowercase();
            img_lower.contains("postgres") || img_lower.contains("redis") || img_lower.contains("mongo") || img_lower.contains("mysql")
        });

        let internal_port = service_data.ports.as_ref().and_then(|p| p.first()).and_then(|p_str| {
            p_str.split(':').last().and_then(|port| port.parse::<i32>().ok())
        }).unwrap_or(3000);

        sqlx::query!(
            "INSERT INTO apps (id, workspace_id, project_id, name, slug, git_repository, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, now(), now())",
            app_id, ws_id, payload.project_id, display_name, slug, service_data.image.unwrap_or_else(|| "built-from-compose".to_string())
        )
        .execute(&mut *tx)
        .await?;

        let instance_id = Uuid::new_v4();
        let container_name = format!("hermes-app-{}", instance_id);
        
        let instance_type_str = if is_addon_db { "preview" } else { "staging" };
        let status_str = "stopped";
        
        sqlx::query!(
            "INSERT INTO app_instances (id, app_id, branch_name, instance_type, status, internal_port, container_name, created_at, updated_at)
             VALUES ($1, $2, 'main', $3::text::app_instance_type, $4::text::app_status, $5, $6, now(), now())",
            instance_id, app_id, instance_type_str, status_str, internal_port, container_name
        )
        .execute(&mut *tx)
        .await?;

        if let Some(env_data) = service_data.environment {
            let mut env_map = HashMap::new();
            match env_data {
                EnvironmentMapping::Map(map) => env_map = map,
                EnvironmentMapping::List(list) => {
                    for line in list {
                        let parts: Vec<&str> = line.splitn(2, '=').collect();
                        if parts.len() == 2 {
                            env_map.insert(parts[0].to_string(), parts[1].to_string());
                        }
                    }
                }
            }

            for (key, val) in env_map {
                if let Ok((encrypted_val, generated_nonce)) = crate::utils::crypto::encrypt_env_value(&val) {
                    sqlx::query!(
                        "INSERT INTO environment_variables (id, workspace_id, project_id, app_instance_id, key, encrypted_value, nonce, is_secret)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                        Uuid::new_v4(), ws_id, payload.project_id, instance_id, key, encrypted_val, generated_nonce, false
                    )
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        if let Some(vols) = service_data.volumes {
            for vol_str in vols {
                let parts: Vec<&str> = vol_str.split(':').collect();
                if parts.len() == 2 {
                    let vol_id = Uuid::new_v4();
                    let host_path = format!("/var/lib/hermes/volumes/{}", vol_id);
                    let container_path = parts[1].to_string();

                    sqlx::query!(
                        "INSERT INTO app_volumes (id, workspace_id, app_id, name, container_path, host_path, created_at)
                         VALUES ($1, $2, $3, $4, $5, $6, now())",
                        vol_id, ws_id, app_id, parts[0].to_string(), container_path, host_path
                    )
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }
    }

    tx.commit().await?;
    Ok(StatusCode::CREATED)
}