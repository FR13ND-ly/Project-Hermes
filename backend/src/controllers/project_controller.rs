use axum::{extract::{State, Path}, http::StatusCode, Json};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::project_dto::{CreateProjectRequest, ProjectResponse, UpdateProjectSettingsRequest, ProjectSettingsResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::project_model::Project;
use crate::utils::error::AppError;

pub async fn create_project(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let slug = payload.name.to_lowercase().trim().replace(" ", "-");

    // Fix: added unwrap_or(false) because query_scalar returns an Option<bool>
    let slug_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE workspace_id = $1 AND slug = $2)",
        ws_id,
        slug
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if slug_exists {
        return Err(AppError::Conflict("A project with this name already exists in this workspace.".to_string()));
    }

    let project_id = Uuid::new_v4();

    // Validate the optional Cloudflare credential belongs to this workspace.
    let cf_cred_id = match payload.cloudflare_credential_id {
        Some(cid) => {
            let ok = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM cloudflare_credentials WHERE id = $1 AND workspace_id = $2)",
                cid, ws_id
            )
            .fetch_one(&state.pool)
            .await?
            .unwrap_or(false);
            if !ok {
                return Err(AppError::NotFound("Cloudflare credential not found in this workspace.".to_string()));
            }
            Some(cid)
        }
        None => None,
    };

    sqlx::query!(
        "INSERT INTO projects (id, workspace_id, name, slug, created_by, cloudflare_credential_id) VALUES ($1, $2, $3, $4, $5, $6)",
        project_id,
        ws_id,
        payload.name,
        slug,
        claims.sub,
        cf_cred_id
    )
    .execute(&state.pool)
    .await?;

    let namespace = std::env::var("HERMES_SYSTEM_NAMESPACE").unwrap_or_else(|_| "hermes-system".to_string());
    let internal_url = format!("http://hermes-backend.{}.svc.cluster.local", namespace);
    let _ = crate::utils::app_env::publish_project_env(
        &state.pool,
        ws_id,
        project_id,
        "HERMES_PLATFORM_URL",
        &internal_url,
        false,
        "platform",
        project_id,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(ProjectResponse {
            id: project_id,
            workspace_id: ws_id,
            name: payload.name,
            slug,
            created_at: chrono::Utc::now(),
        }),
    ))
}

pub async fn list_workspace_projects(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<ProjectResponse>>, AppError> {
    let ws_id = match claims.current_workspace_id {
        Some(id) => id,
        None => return Ok(Json(vec![])),
    };

    // Per SQLx 0.7+: the parameter is passed separately using .bind()
    let projects = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE workspace_id = $1 ORDER BY created_at DESC"
    )
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    let response = projects
        .into_iter()
        .map(|p| ProjectResponse {
            id: p.id,
            workspace_id: p.workspace_id,
            name: p.name,
            slug: p.slug,
            created_at: p.created_at,
        })
        .collect();

    Ok(Json(response))
}

pub async fn get_project(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<ProjectResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let project = sqlx::query_as::<_, Project>(
        "SELECT * FROM projects WHERE id = $1 AND workspace_id = $2"
    )
    .bind(project_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Project not found.".to_string()))?;

    Ok(Json(ProjectResponse {
        id: project.id,
        workspace_id: project.workspace_id,
        name: project.name,
        slug: project.slug,
        created_at: project.created_at,
    }))
}

pub async fn delete_project(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Check if the project exists and belongs to the workspace
    let project = sqlx::query!(
        "SELECT id, name FROM projects WHERE id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Project not found.".to_string()))?;

    // Get all apps in the project
    let apps = sqlx::query!(
        "SELECT id, name FROM apps WHERE project_id = $1 AND workspace_id = $2",
        project.id, ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let namespace = format!("hermes-ws-{}", ws_id);

    // Collect all container names for all app instances in the project
    let mut containers_to_delete = Vec::new();
    for app in &apps {
        let instances = sqlx::query!(
            "SELECT container_name, assigned_domain FROM app_instances WHERE app_id = $1",
            app.id
        )
        .fetch_all(&state.pool)
        .await?;
        for inst in instances {
            containers_to_delete.push((inst.container_name, inst.assigned_domain));
        }
    }

    // Get all databases in the project
    let databases = sqlx::query!(
        "SELECT id, container_name FROM databases WHERE project_id = $1 AND workspace_id = $2",
        project.id, ws_id
    )
    .fetch_all(&state.pool)
    .await?;

    let db_ids_to_clean = databases.iter().map(|db| db.id).collect::<Vec<_>>();
    let db_containers_to_delete = databases
        .into_iter()
        .map(|db| db.container_name)
        .collect::<Vec<_>>();

    // Get all serverless instances in the project (their K8s services live in the workspace namespace)
    let serverless_names = sqlx::query!(
        "SELECT name FROM serverless_instances WHERE project_id = $1",
        project.id
    )
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|f| f.name)
    .collect::<Vec<_>>();

    // Collect every domain attached to this project's resources (apps, serverless
    // functions, databases) so their external resources can be torn down too.
    let project_domains = sqlx::query_as::<_, crate::models::domain_model::Domain>(
        "SELECT * FROM domains WHERE workspace_id = $1 AND (
            target_id IN (SELECT id FROM app_instances WHERE app_id IN (SELECT id FROM apps WHERE project_id = $2))
            OR target_id IN (SELECT id FROM serverless_instances WHERE project_id = $2)
            OR target_id IN (SELECT id FROM databases WHERE project_id = $2)
        )"
    )
    .bind(ws_id)
    .bind(project.id)
    .fetch_all(&state.pool)
    .await?;

    // Delete Kubernetes resources asynchronously
    tokio::spawn(async move {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            // Delete app instances
            for (container_name, assigned_domain) in containers_to_delete {
                if assigned_domain.is_some() {
                    let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &container_name).await;
                }
                let _ = crate::utils::k8s::K8sManager::delete_app(&k8s_client, &namespace, &container_name).await;
                let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, &container_name).await;
            }

            // Delete databases
            for db_container in db_containers_to_delete {
                let _ = crate::utils::k8s::K8sManager::delete_database(&k8s_client, &namespace, &db_container).await;
            }

            // Tear down serverless functions (Knative service, ingress and proxy resources)
            for fn_name in serverless_names {
                let svc = format!("fn-{}", crate::controllers::serverless_controller::slugify(&fn_name));
                let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, &svc).await;
                let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &svc).await;

                use kube::api::{Api, DeleteParams};
                let configmaps: Api<k8s_openapi::api::core::v1::ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
                let _ = configmaps.delete(&format!("{}-proxy-config", svc), &DeleteParams::default()).await;
                let deployments: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(k8s_client.clone(), &namespace);
                let _ = deployments.delete(&format!("{}-proxy", svc), &DeleteParams::default()).await;
                let services: Api<k8s_openapi::api::core::v1::Service> = Api::namespaced(k8s_client.clone(), &namespace);
                let _ = services.delete(&format!("{}-external", svc), &DeleteParams::default()).await;
                let _ = services.delete(&format!("{}-proxy-svc", svc), &DeleteParams::default()).await;
            }
        }
    });

    // Remove physical database backup directories on host disk
    for db_id in &db_ids_to_clean {
        let db_backup_path = format!("/var/lib/hermes/backups/{}", db_id);
        let _ = std::fs::remove_dir_all(db_backup_path);
    }

    // Remove physical storage bucket files for this project (objects + bucket dirs).
    crate::controllers::storage_controller::purge_project_buckets_physical(&state.pool, ws_id, project.id).await;

    // Tear down each domain's external resources (Cloudflare DNS, nginx, ingress).
    for domain in &project_domains {
        crate::controllers::domain_controller::teardown_domain_resources(&state.pool, ws_id, domain).await;
    }

    // Delete all dependent rows manually since there are no foreign key ON DELETE CASCADE on apps/databases
    let mut tx = state.pool.begin().await?;

    // 0. Delete domains pointing at this project's resources, and the project's
    //    storage buckets (+ their objects and any cron jobs targeting them).
    //    storage_buckets.project_id is ON DELETE SET NULL, so they must be removed
    //    explicitly or they would be orphaned in the workspace.
    sqlx::query!(
        "DELETE FROM domains WHERE workspace_id = $1 AND (
            target_id IN (SELECT id FROM app_instances WHERE app_id IN (SELECT id FROM apps WHERE project_id = $2))
            OR target_id IN (SELECT id FROM serverless_instances WHERE project_id = $2)
            OR target_id IN (SELECT id FROM databases WHERE project_id = $2)
        )",
        ws_id, project.id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "DELETE FROM cron_jobs WHERE target_type = 'storage' AND target_id IN (SELECT id FROM storage_buckets WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "DELETE FROM storage_objects WHERE bucket_id IN (SELECT id FROM storage_buckets WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "DELETE FROM storage_buckets WHERE project_id = $1",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 1. Delete app incidents
    sqlx::query!(
        "DELETE FROM app_incident_logs WHERE app_instance_id IN (
            SELECT id FROM app_instances WHERE app_id IN (
                SELECT id FROM apps WHERE project_id = $1
            )
        )",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 2. Delete environment variables for every instance of every app in the project
    sqlx::query!(
        "DELETE FROM environment_variables WHERE app_instance_id IN (
            SELECT id FROM app_instances WHERE app_id IN (
                SELECT id FROM apps WHERE project_id = $1
            )
        )",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 3. Delete app volumes
    sqlx::query!(
        "DELETE FROM app_volumes WHERE app_id IN (SELECT id FROM apps WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 4. Delete app builds
    sqlx::query!(
        "DELETE FROM app_builds WHERE app_id IN (SELECT id FROM apps WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 5. Delete BaaS services for the project (cascades their users/roles/api-keys/
    //    refresh-tokens). BaaS is now a standalone project resource, not app-scoped.
    sqlx::query!(
        "DELETE FROM baas_services WHERE project_id = $1",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 6. Delete cron jobs
    sqlx::query!(
        "DELETE FROM cron_jobs WHERE project_id = $1 OR app_id IN (SELECT id FROM apps WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 7. Delete app instances
    sqlx::query!(
        "DELETE FROM app_instances WHERE app_id IN (SELECT id FROM apps WHERE project_id = $1)",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 8. Delete apps
    sqlx::query!(
        "DELETE FROM apps WHERE project_id = $1",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 9. Delete databases
    sqlx::query!(
        "DELETE FROM databases WHERE project_id = $1",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    // 10. Finally, delete the project
    sqlx::query!(
        "DELETE FROM projects WHERE id = $1",
        project.id
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_project_ssh_keys(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<crate::dtos::project_ssh_key_dto::ProjectSshKeyResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to current workspace
    let project_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        project_id,
        ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !project_exists {
        return Err(AppError::NotFound("Project not found.".to_string()));
    }

    let keys = sqlx::query!(
        "SELECT id, name, host, public_key, created_at FROM project_ssh_keys WHERE project_id = $1 ORDER BY created_at DESC",
        project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let response = keys
        .into_iter()
        .map(|k| crate::dtos::project_ssh_key_dto::ProjectSshKeyResponse {
            id: k.id,
            project_id,
            name: k.name,
            host: k.host,
            public_key: k.public_key,
            created_at: k.created_at,
        })
        .collect();

    Ok(Json(response))
}

pub async fn create_project_ssh_key(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<crate::dtos::project_ssh_key_dto::CreateSshKeyRequest>,
) -> Result<(StatusCode, Json<crate::dtos::project_ssh_key_dto::ProjectSshKeyResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to workspace
    let project_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        project_id,
        ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !project_exists {
        return Err(AppError::NotFound("Project not found.".to_string()));
    }

    let name = payload.name.trim().to_string();
    let host = payload.host.trim().to_lowercase();

    if name.is_empty() || host.is_empty() {
        return Err(AppError::Validation("Name and host cannot be empty.".to_string()));
    }

    // Check unique host/name per project
    let name_or_host_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_ssh_keys WHERE project_id = $1 AND (name = $2 OR host = $3))",
        project_id,
        name,
        host
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if name_or_host_exists {
        return Err(AppError::Conflict("An SSH key with this name or host already exists for this project.".to_string()));
    }

    // Determine private & public key
    let (private_key, public_key) = match payload.private_key {
        Some(ref pk) if !pk.trim().is_empty() => {
            let temp_dir_path = std::path::PathBuf::from(format!("./.tmp_ssh_import_{}", Uuid::new_v4()));
            let _ = std::fs::create_dir_all(&temp_dir_path);
            let key_file = temp_dir_path.join("id_git");
            let _ = std::fs::write(&key_file, pk.trim());
            
            let mut cmd = std::process::Command::new("ssh-keygen");
            cmd.args(&["-y", "-f", &key_file.to_string_lossy()]);
            
            let pub_key = if let Ok(out) = cmd.output() {
                if out.status.success() {
                    String::from_utf8_lossy(&out.stdout).trim().to_string()
                } else {
                    "ssh-rsa IMPORTED_KEY".to_string()
                }
            } else {
                "ssh-rsa IMPORTED_KEY".to_string()
            };
            
            let _ = std::fs::remove_dir_all(&temp_dir_path);
            (pk.trim().to_string(), pub_key)
        }
        _ => {
            crate::utils::ssh::generate_ssh_keypair()?
        }
    };

    // Encrypt private key
    let (encrypted_private_key, nonce) = crate::utils::crypto::encrypt_env_value(&private_key)?;

    let key_id = Uuid::new_v4();
    let created_at = chrono::Utc::now();

    sqlx::query!(
        "INSERT INTO project_ssh_keys (id, project_id, name, host, encrypted_private_key, nonce, public_key, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        key_id,
        project_id,
        name,
        host,
        encrypted_private_key,
        nonce,
        public_key,
        created_at
    )
    .execute(&state.pool)
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(crate::dtos::project_ssh_key_dto::ProjectSshKeyResponse {
            id: key_id,
            project_id,
            name,
            host,
            public_key,
            created_at,
        })
    ))
}

pub async fn delete_project_ssh_key(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, key_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project belongs to workspace
    let project_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        project_id,
        ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !project_exists {
        return Err(AppError::NotFound("Project not found.".to_string()));
    }

    let rows_affected = sqlx::query!(
        "DELETE FROM project_ssh_keys WHERE id = $1 AND project_id = $2",
        key_id,
        project_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Err(AppError::NotFound("SSH key not found.".to_string()));
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Build the (token-masked) settings response for a project the caller owns.
async fn fetch_project_settings(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    ws_id: Uuid,
) -> Result<ProjectSettingsResponse, AppError> {
    let p = sqlx::query!(
        "SELECT cloudflare_credential_id, ingress_ip, base_domain
         FROM projects WHERE id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Project not found in this workspace.".to_string()))?;

    Ok(ProjectSettingsResponse {
        cloudflare_credential_id: p.cloudflare_credential_id,
        ingress_ip: p.ingress_ip,
        base_domain: p.base_domain,
    })
}

pub async fn get_project_settings(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
) -> Result<Json<ProjectSettingsResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    Ok(Json(fetch_project_settings(&state.pool, project_id, ws_id).await?))
}

pub async fn update_project_settings(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<UpdateProjectSettingsRequest>,
) -> Result<Json<ProjectSettingsResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Validate the selected Cloudflare credential (if any) belongs to this workspace;
    // the form always sends the current selection, so null means "no Cloudflare".
    if let Some(cid) = payload.cloudflare_credential_id {
        let ok = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM cloudflare_credentials WHERE id = $1 AND workspace_id = $2)",
            cid, ws_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
        if !ok {
            return Err(AppError::NotFound("Cloudflare credential not found in this workspace.".to_string()));
        }
    }

    let ingress_ip = payload.ingress_ip.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let base_domain = payload.base_domain.as_deref().map(str::trim).filter(|s| !s.is_empty());

    let rows = sqlx::query!(
        "UPDATE projects SET
            cloudflare_credential_id = $1,
            ingress_ip = $2,
            base_domain = $3
         WHERE id = $4 AND workspace_id = $5",
        payload.cloudflare_credential_id, ingress_ip, base_domain, project_id, ws_id
    )
    .execute(&state.pool)
    .await?
    .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    Ok(Json(fetch_project_settings(&state.pool, project_id, ws_id).await?))
}