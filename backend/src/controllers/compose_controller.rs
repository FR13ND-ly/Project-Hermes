use axum::{extract::State, http::StatusCode, Json};
use uuid::Uuid;
use std::collections::HashMap;

use crate::app_state::AppState;
use crate::dtos::compose_dto::{
    ImportComposeRequest, ComposeStack, ComposeService, EnvironmentMapping,
    ComposePlan, PlanApp, PlanDatabase, PlanVolume, PlanEnv, PlanRequest, ApplyPlanRequest,
};
use crate::models::database_model::DbType;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;

/// Classify a compose image as a managed database type → (type, default port).
fn db_kind_from_image(image: &str) -> Option<(&'static str, i32)> {
    let base = image.split(':').next().unwrap_or(image).rsplit('/').next().unwrap_or(image).to_lowercase();
    if base.contains("postgres") { Some(("postgres", 5432)) }
    else if base.contains("mariadb") || base.contains("mysql") { Some(("mysql", 3306)) }
    else if base.contains("redis") { Some(("redis", 6379)) }
    else if base.contains("mongo") { Some(("mongodb", 27017)) }
    else { None }
}

fn image_tag(image: &str) -> String {
    image.split_once(':').map(|(_, t)| t.to_string()).unwrap_or_else(|| "latest".to_string())
}

fn env_pairs(env: &Option<EnvironmentMapping>) -> Vec<PlanEnv> {
    let mut out = Vec::new();
    if let Some(e) = env {
        match e {
            EnvironmentMapping::Map(m) => for (k, v) in m { out.push(PlanEnv { key: k.clone(), value: v.clone() }); },
            EnvironmentMapping::List(l) => for line in l {
                if let Some((k, v)) = line.split_once('=') { out.push(PlanEnv { key: k.to_string(), value: v.to_string() }); }
            },
        }
    }
    out
}

fn build_path_of(svc: &ComposeService) -> Option<String> {
    let b = svc.build.as_ref()?;
    match b {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Mapping(m) => m.get("context")
            .and_then(|v| v.as_str()).map(|s| s.to_string()),
        _ => None,
    }
}

fn first_port(svc: &ComposeService, default: i32) -> i32 {
    svc.ports.as_ref().and_then(|p| p.first())
        .and_then(|s| s.split(':').last().and_then(|p| p.trim().parse::<i32>().ok()))
        .unwrap_or(default)
}

fn external_port_from_compose(svc: &ComposeService) -> Option<i32> {
    let port_str = svc.ports.as_ref()?.first()?;
    let parts: Vec<&str> = port_str.split(':').collect();
    if parts.len() >= 2 {
        parts[0].trim().parse::<i32>().ok()
    } else {
        None
    }
}

fn volumes_of(service: &str, svc: &ComposeService) -> Vec<PlanVolume> {
    let mut out = Vec::new();
    if let Some(vols) = &svc.volumes {
        for v in vols {
            let parts: Vec<&str> = v.split(':').collect();
            if parts.len() >= 2 {
                out.push(PlanVolume { service: service.to_string(), name: parts[0].to_string(), container_path: parts[1].to_string() });
            }
        }
    }
    out
}

/// Build a split plan from a docker-compose file (no DB writes).
fn plan_from_compose(yaml: &str) -> Result<ComposePlan, AppError> {
    let stack: ComposeStack = serde_yaml::from_str(yaml)
        .map_err(|e| AppError::Validation(format!("docker-compose YAML invalid: {}", e)))?;

    let mut apps = Vec::new();
    let mut databases = Vec::new();

    for (service, svc) in &stack.services {
        let depends_on = svc.depends_on.as_ref().map(|d| d.names()).unwrap_or_default();

        if let Some((db_type, port)) = svc.image.as_deref().and_then(db_kind_from_image) {
            databases.push(PlanDatabase {
                service: service.clone(),
                name: service.clone(),
                db_type: db_type.to_string(),
                version: svc.image.as_deref().map(image_tag).unwrap_or_else(|| "latest".to_string()),
                internal_port: port,
                include: true,
            });
            continue;
        }

        let build_path = build_path_of(svc);
        let buildable = build_path.is_some();
        apps.push(PlanApp {
            service: service.clone(),
            name: service.clone(),
            image: svc.image.clone(),
            build_path,
            internal_port: first_port(svc, 3000),
            external_port: external_port_from_compose(svc),
            buildable,
            env: env_pairs(&svc.environment),
            volumes: volumes_of(service, svc),
            depends_on,
            // image-only non-DB services can't be built by Hermes → default off.
            include: buildable,
        });
    }

    apps.sort_by(|a, b| a.service.cmp(&b.service));
    databases.sort_by(|a, b| a.service.cmp(&b.service));
    Ok(ComposePlan { apps, databases })
}

/// POST /stacks/plan — preview the auto-split (no resources created).
pub async fn plan_compose(
    State(_state): State<AppState>,
    AuthenticatedUser(_claims): AuthenticatedUser,
    Json(payload): Json<PlanRequest>,
) -> Result<Json<ComposePlan>, AppError> {
    Ok(Json(plan_from_compose(&payload.compose_yaml)?))
}

/// POST /stacks/apply — create the (possibly edited) plan: real databases, apps,
/// volumes, env, and depends_on links.
pub async fn apply_compose_plan(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<ApplyPlanRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let project_ok = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        payload.project_id, ws_id
    ).fetch_one(&state.pool).await?.unwrap_or(false);
    if !project_ok {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let branch = payload.branch_name.clone().unwrap_or_else(|| "main".to_string());

    // 1. Databases — service name → published project_env id (for depends_on links).
    let mut db_env_by_service: HashMap<String, Uuid> = HashMap::new();
    for db in payload.plan.databases.iter().filter(|d| d.include) {
        let db_type = match db.db_type.as_str() {
            "postgres" => DbType::Postgres,
            "mysql" => DbType::Mysql,
            "redis" => DbType::Redis,
            "mongodb" => DbType::Mongodb,
            _ => continue,
        };
        let db_id = Uuid::new_v4();
        let raw_password = crate::utils::string_gen::generate_secure_string(24);
        let (enc_pw, pw_nonce) = crate::utils::crypto::encrypt_env_value(&raw_password)?;
        let db_user = format!("hermes_user_{}", crate::utils::string_gen::generate_secure_string(5).to_lowercase());
        let db_name = format!("hermes_db_{}", crate::utils::string_gen::generate_secure_string(5).to_lowercase());
        let type_str = db.db_type.clone();
        let container_name = format!("h-db-{}-{}", type_str, &db_id.to_string()[..8]);
        let full_image = format!("{}:{}", type_str, db.version);

        let connection_url = match db_type {
            DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db_user, raw_password, container_name, db.internal_port, db_name),
            DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db_user, raw_password, container_name, db.internal_port, db_name),
            DbType::Redis => format!("redis://{}:{}", container_name, db.internal_port),
            DbType::Mongodb => format!("mongodb://{}:{}@{}:{}", db_user, raw_password, container_name, db.internal_port),
        };

        sqlx::query!(
            "INSERT INTO databases (id, workspace_id, project_id, name, type, version, db_user, db_password, db_password_nonce, db_name, container_name, internal_port, is_external, status, cpu_limit, memory_limit_mb)
             VALUES ($1, $2, $3, $4, $5::text::db_type, $6, $7, $8, $9, $10, $11, $12, false, 'provisioning'::db_status, 0, 0)",
            db_id, ws_id, payload.project_id, db.name, type_str, full_image, db_user, enc_pw, pw_nonce, db_name, container_name, db.internal_port
        )
        .execute(&state.pool)
        .await?;

        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::DatabaseStatusChanged {
                workspace_id: ws_id, database_id: db_id, container_name: container_name.clone(), status: "provisioning".to_string(),
            }
        );

        // Publish connection string to the project pool (disambiguate the key).
        let taken = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE project_id = $1 AND key = 'DATABASE_URL')",
            payload.project_id
        ).fetch_one(&state.pool).await?.unwrap_or(false);
        let key = if taken {
            format!("{}_DATABASE_URL", crate::utils::app_env::sanitize_key_fragment(&db.name, "DB"))
        } else {
            "DATABASE_URL".to_string()
        };
        if let Ok(env_id) = crate::utils::app_env::publish_project_env(
            &state.pool, ws_id, payload.project_id, &key, &connection_url, true, "database", db_id,
        ).await {
            db_env_by_service.insert(db.service.clone(), env_id);
        }

        crate::controllers::database_controller::spawn_db_provisioning(
            state.pool.clone(), ws_id, db_id, db_type, container_name, full_image, db_user, raw_password, db_name, db.internal_port, 0, 0, false, None,
        );
    }

    // 2. Apps + their env/volumes, then collect depends_on links.
    for app in payload.plan.apps.iter().filter(|a| a.include) {
        let app_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        let slug = app.name.trim().to_lowercase().replace([' ', '_'], "-");
        let container_name = crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}-{}", slug, branch, &instance_id.to_string()[..8]));
        let git_repository = payload.git_repository.clone().unwrap_or_else(|| app.image.clone().unwrap_or_else(|| "compose".to_string()));
        let can_build = app.buildable && payload.git_repository.is_some();

        sqlx::query!(
            "INSERT INTO apps (id, workspace_id, project_id, name, slug, git_repository, git_subpath, git_credential_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            app_id, ws_id, payload.project_id, app.name, slug, git_repository, app.build_path, payload.git_credential_id
        )
        .execute(&state.pool)
        .await?;

        let status_str = if can_build { "building" } else { "stopped" };
        let ext_port = if let Some(p) = app.external_port {
            let in_use = sqlx::query_scalar!(
                "SELECT EXISTS(
                    SELECT 1 FROM app_instances WHERE external_port = $1
                    UNION
                    SELECT 1 FROM databases WHERE external_port = $1 AND is_external = true
                    UNION
                    SELECT 1 FROM serverless_instances WHERE external_port = $1
                )",
                p
            )
            .fetch_one(&state.pool)
            .await?
            .unwrap_or(false);
            if in_use {
                crate::controllers::app_controller::get_random_available_port(&state.pool).await?
            } else {
                p
            }
        } else {
            crate::controllers::app_controller::get_random_available_port(&state.pool).await?
        };

        sqlx::query!(
            "INSERT INTO app_instances (id, app_id, branch_name, instance_type, status, internal_port, container_name, external_port)
             VALUES ($1, $2, $3, 'staging'::app_instance_type, $4::text::app_status, $5, $6, $7)",
            instance_id, app_id, branch, status_str, app.internal_port, container_name, ext_port
        )
        .execute(&state.pool)
        .await?;

        for e in &app.env {
            let key = e.key.trim().to_uppercase().replace(' ', "_");
            if key.is_empty() { continue; }

            // Check if it already exists in the project env variables pool
            let project_env = sqlx::query!(
                "SELECT id FROM project_env_variables WHERE project_id = $1 AND key = $2",
                payload.project_id, key
            )
            .fetch_optional(&state.pool)
            .await?;

            if let Some(pe) = project_env {
                let _ = sqlx::query!(
                    "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    instance_id, pe.id
                ).execute(&state.pool).await;
            } else {
                if let Ok((enc, nonce)) = crate::utils::crypto::encrypt_env_value(&e.value) {
                    let _ = sqlx::query!(
                        "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
                         VALUES ($1, $2, $3, $4, $5, $6, false) ON CONFLICT (app_instance_id, key) DO NOTHING",
                        Uuid::new_v4(), ws_id, instance_id, key, enc, nonce
                    ).execute(&state.pool).await;
                }
            }
        }

        for vol in &app.volumes {
            let vol_id = Uuid::new_v4();
            let host_path = format!("/var/lib/hermes/volumes/{}", vol_id);
            let _ = sqlx::query!(
                "INSERT INTO app_volumes (id, workspace_id, app_id, name, container_path, host_path, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6, now())",
                vol_id, ws_id, app_id, vol.name, vol.container_path, host_path
            ).execute(&state.pool).await;
        }

        // Link this app to the connection env of every database it depends on.
        for dep in &app.depends_on {
            if let Some(env_id) = db_env_by_service.get(dep) {
                let _ = sqlx::query!(
                    "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    instance_id, env_id
                ).execute(&state.pool).await;
            }
        }

        if can_build {
            let pool = state.pool.clone();
            let repo = git_repository.clone();
            let br = branch.clone();
            tokio::spawn(async move {
                crate::utils::builder::run_ephemeral_build(pool, instance_id, repo, br, None).await;
            });
        }
    }

    Ok(StatusCode::CREATED)
}

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
                let clean_key = key.trim().to_uppercase().replace(' ', "_");
                if clean_key.is_empty() { continue; }

                let project_env = sqlx::query!(
                    "SELECT id FROM project_env_variables WHERE project_id = $1 AND key = $2",
                    payload.project_id, clean_key
                )
                .fetch_optional(&mut *tx)
                .await?;

                if let Some(pe) = project_env {
                    sqlx::query!(
                        "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                        instance_id, pe.id
                    )
                    .execute(&mut *tx)
                    .await?;
                } else {
                    if let Ok((encrypted_val, generated_nonce)) = crate::utils::crypto::encrypt_env_value(&val) {
                        sqlx::query!(
                            "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
                             VALUES ($1, $2, $3, $4, $5, $6, $7)",
                            Uuid::new_v4(), ws_id, instance_id, clean_key, encrypted_val, generated_nonce, false
                        )
                        .execute(&mut *tx)
                        .await?;
                    }
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
