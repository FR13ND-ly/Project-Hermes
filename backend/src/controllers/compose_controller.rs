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
            enable_storage: false,
            // Defaults: auto network name (None), publish URL on, auto key (<SERVICE>_URL).
            network_name: None,
            publish_url: Some(true),
            url_env_key: None,
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

    // Validate service aliases are unique (within this stack + across the workspace)
    // before creating anything — services share the namespace.
    let mut seen_aliases: Vec<String> = Vec::new();
    for app in payload.plan.apps.iter().filter(|a| a.include) {
        let slug = app.name.trim().to_lowercase().replace([' ', '_'], "-");
        let alias = match app.network_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(custom) => crate::utils::string_gen::sanitize_k8s_name(custom),
            None => crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}", slug, branch)),
        };
        if seen_aliases.contains(&alias) {
            return Err(AppError::Validation(format!("Numele de serviciu '{}' apare de două ori în acest stack.", alias)));
        }
        seen_aliases.push(alias.clone());
        let taken = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE a.workspace_id = $1 AND ai.network_alias = $2)",
        )
        .bind(ws_id)
        .bind(&alias)
        .fetch_one(&state.pool)
        .await?;
        if taken {
            return Err(AppError::Conflict(format!("Numele de serviciu '{}' e deja folosit în acest workspace. Alege altul.", alias)));
        }
    }

    // 2. Pass 1: Create apps in DB + conditionally generate BaaS auth secrets.
    let mut app_identities: HashMap<String, (Uuid, Uuid)> = HashMap::new();
    // service name -> published project_env id for that app's own URL (depends_on links).
    let mut app_url_env_by_service: HashMap<String, Uuid> = HashMap::new();
    // service name -> chosen in-cluster network alias (persisted on the instance in Pass 2).
    let mut app_alias_by_service: HashMap<String, String> = HashMap::new();
    for app in payload.plan.apps.iter().filter(|a| a.include) {
        let app_id = Uuid::new_v4();
        let instance_id = Uuid::new_v4();
        app_identities.insert(app.service.clone(), (app_id, instance_id));

        let slug = app.name.trim().to_lowercase().replace([' ', '_'], "-");
        let git_repository = payload.git_repository.clone().unwrap_or_else(|| app.image.clone().unwrap_or_else(|| "compose".to_string()));

        sqlx::query!(
            "INSERT INTO apps (id, workspace_id, project_id, name, slug, git_repository, git_subpath, git_credential_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            app_id, ws_id, payload.project_id, app.name, slug, git_repository, app.build_path, payload.git_credential_id
        )
        .execute(&state.pool)
        .await?;

        // Resolve this app's in-cluster network alias (custom or auto) and optionally
        // publish its URL into the project env pool, e.g. service "backend" ->
        // BACKEND_URL = http://backend:3000. depends_on consumers auto-linked in Pass 2.
        let network_alias = match app.network_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(custom) => crate::utils::string_gen::sanitize_k8s_name(custom),
            None => crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}", slug, branch)),
        };
        app_alias_by_service.insert(app.service.clone(), network_alias.clone());
        if app.publish_url != Some(false) {
            let app_url = format!("http://{}:{}", network_alias, app.internal_port);
            let url_key = app.url_env_key.as_deref().map(str::trim).filter(|s| !s.is_empty())
                .map(|s| s.to_uppercase())
                .unwrap_or_else(|| format!("{}_URL", crate::utils::app_env::sanitize_key_fragment(&app.service, "APP")));
            if let Ok(env_id) = crate::utils::app_env::publish_project_env(
                &state.pool, ws_id, payload.project_id, &url_key, &app_url, false, "app", app_id,
            ).await {
                app_url_env_by_service.insert(app.service.clone(), env_id);
            }
        }

    }

    // 3. Pass 2: Create app instances, map env vars (links & local), volumes, database links, and trigger builds.
    for app in payload.plan.apps.iter().filter(|a| a.include) {
        let (app_id, instance_id) = match app_identities.get(&app.service) {
            Some(&ids) => ids,
            None => continue,
        };

        let slug = app.name.trim().to_lowercase().replace([' ', '_'], "-");
        let container_name = crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}-{}", slug, branch, &instance_id.to_string()[..8]));
        let git_repository = payload.git_repository.clone().unwrap_or_else(|| app.image.clone().unwrap_or_else(|| "compose".to_string()));
        let can_build = app.buildable && payload.git_repository.is_some();

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

        // Persist the chosen network alias (resolved in Pass 1) for the deploy path.
        if let Some(alias) = app_alias_by_service.get(&app.service) {
            let _ = sqlx::query("UPDATE app_instances SET network_alias = $1 WHERE id = $2")
                .bind(alias)
                .bind(instance_id)
                .execute(&state.pool)
                .await;
        }

        // BaaS for compose services is provisioned once in Pass 1 (standalone service);
        // nothing app-scoped to link here anymore.

        // Provision + link a private storage bucket for this service, if requested.
        if app.enable_storage {
            if let Err(e) = crate::controllers::storage_controller::provision_bucket_for_instance(
                &state.pool, ws_id, payload.project_id, claims.sub, &app.name, instance_id,
            ).await {
                tracing::warn!(service = %app.service, "Failed to provision storage bucket in compose Pass 2: {}", e);
            }
        }

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

        // Link this app to the connection env of every database AND app it depends on
        // (depends_on: backend  ->  this app gets BACKEND_URL, auto-wired).
        for dep in &app.depends_on {
            if let Some(env_id) = db_env_by_service.get(dep).or_else(|| app_url_env_by_service.get(dep)) {
                let _ = sqlx::query!(
                    "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    instance_id, env_id
                ).execute(&state.pool).await;
            }
        }

        if can_build {
            let _ = crate::utils::job_queue::enqueue_build(
                &state.pool, instance_id, git_repository.clone(), branch.clone(), None,
            ).await;
        }
    }

    // Auto-register the GitHub push webhook for the repo backing this stack, so
    // compose/split-created apps also get auto-deploy on push (this path used to
    // skip it entirely). One webhook per repo (all services share git_repository).
    if let Some(ref repo) = payload.git_repository {
        let host = std::env::var("HERMES_BASE_DOMAIN").unwrap_or_default();
        crate::controllers::app_controller::try_register_github_webhook(
            &state.pool, ws_id, claims.sub, payload.git_credential_id, repo, &host,
        ).await;
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
