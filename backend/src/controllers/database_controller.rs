use axum::{
    extract::{State, Path, Query},
    http::StatusCode,
    Json,
};
use axum::response::sse::{Event, Sse};
use futures_util::{Stream, StreamExt};
use std::convert::Infallible;
use uuid::Uuid;

use crate::app_state::AppState;
use crate::models::database_model::{DatabaseService, DbType, DbStatus};
use crate::dtos::database_dto::{CreateDatabaseRequest, DatabaseResponse};
use crate::dtos::database_backup_dto::BackupResponse;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{crypto, string_gen, error::AppError};

pub async fn create_database(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateDatabaseRequest>,
) -> Result<(StatusCode, Json<DatabaseResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_id = Uuid::new_v4();
    let raw_password = string_gen::generate_secure_string(24);
    let (encrypted_password, password_nonce) = crypto::encrypt_env_value(&raw_password)?;
    
    let db_user = format!("hermes_user_{}", string_gen::generate_secure_string(5).to_lowercase());
    let db_name = format!("hermes_db_{}", string_gen::generate_secure_string(5).to_lowercase());
    
    let type_str = match payload.r#type {
        DbType::Postgres => "postgres",
        DbType::Mysql => "mysql",
        DbType::Redis => "redis",
        DbType::Mongodb => "mongodb",
    };
    
    let container_name = format!("h-db-{}-{}", type_str, &db_id.to_string()[..8]);
    
    let (image_name, version) = match payload.r#type {
        DbType::Postgres => ("postgres", payload.version.clone().unwrap_or_else(|| "alpine".to_string())),
        DbType::Redis => ("redis", payload.version.clone().unwrap_or_else(|| "alpine".to_string())),
        DbType::Mongodb => ("mongo", payload.version.clone().unwrap_or_else(|| "6.0".to_string())),
        DbType::Mysql => ("mysql", payload.version.clone().unwrap_or_else(|| "8.0".to_string())),
    };
    let full_image = format!("{}:{}", image_name, version);
    
    let internal_port = match payload.r#type {
        DbType::Postgres => 5432,
        DbType::Mysql => 3306,
        DbType::Redis => 6379,
        DbType::Mongodb => 27017,
    };

    let connection_url = match payload.r#type {
        DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db_user, raw_password, container_name, internal_port, db_name),
        DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db_user, raw_password, container_name, internal_port, db_name),
        DbType::Redis => format!("redis://:{}@{}:{}", raw_password, container_name, internal_port),
        DbType::Mongodb => format!("mongodb://{}:{}@{}:{}/?authSource=admin", db_user, raw_password, container_name, internal_port),
    };

    let is_external = payload.is_external.unwrap_or(false);
    let ext_port = if is_external {
        Some(payload.external_port.unwrap_or(internal_port))
    } else {
        None
    };

    // --- Resource availability check BEFORE creating the DB record ---
    let db_memory_needed_mb = payload.memory_limit_mb.filter(|&m| m > 0).unwrap_or(256) as i64;
    crate::utils::limits::check_workspace_memory_limit(
        &state.pool,
        ws_id,
        db_memory_needed_mb,
        None
    ).await?;
    let db_cpu_needed = payload.cpu_limit.filter(|&c| c > 0).unwrap_or(0);
    if db_cpu_needed > 0 {
        crate::utils::limits::check_workspace_cpu_limit(&state.pool, ws_id, db_cpu_needed, None).await?;
    }
    // --- End resource check ---

    sqlx::query!(
        "INSERT INTO databases (id, workspace_id, project_id, app_instance_id, name, type, version, db_user, db_password, db_password_nonce, db_name, container_name, internal_port, is_external, external_port, status, cpu_limit, memory_limit_mb, storage_size_gb)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)",
        db_id, ws_id, payload.project_id, payload.app_instance_id, payload.name, payload.r#type.clone() as DbType, full_image, db_user, encrypted_password, password_nonce, db_name, container_name, internal_port, is_external, ext_port, DbStatus::Provisioning as DbStatus, payload.cpu_limit.unwrap_or(0), payload.memory_limit_mb.unwrap_or(0), payload.storage_size_gb.filter(|&s| s > 0).unwrap_or(1)
    )
    .execute(&state.pool)
    .await?;

    // Announce provisioning so the global build indicator picks it up immediately.
    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::DatabaseStatusChanged {
            workspace_id: ws_id,
            database_id: db_id,
            container_name: container_name.clone(),
            status: "provisioning".to_string(),
        }
    );

    // Publish the connection string into the project's env pool so any app in the
    // project can opt into it. Opt-out via publish_to_env=false. The key defaults to
    // DATABASE_URL (disambiguated by name if another database already published it)
    // but a custom env_key may be supplied at creation.
    if payload.publish_to_env.unwrap_or(true) {
        let url_taken = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM project_env_variables
             WHERE project_id = $1 AND key = 'DATABASE_URL' AND source_id IS DISTINCT FROM $2)",
            payload.project_id,
            db_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
        let default_key = if url_taken {
            format!(
                "{}_DATABASE_URL",
                crate::utils::app_env::sanitize_key_fragment(&payload.name, &type_str.to_uppercase())
            )
        } else {
            "DATABASE_URL".to_string()
        };
        let env_key = match payload.env_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(custom) => crate::utils::app_env::sanitize_key_fragment(custom, &default_key),
            None => default_key,
        };
        let project_env_id = crate::utils::app_env::publish_project_env(
            &state.pool, ws_id, payload.project_id, &env_key, &connection_url, true, "database", db_id,
        )
        .await?;

        // If the database was created bound to a specific app instance, opt that
        // instance into the new var straight away (existing bind UX preserved).
        if let Some(instance_id) = payload.app_instance_id {
            sqlx::query!(
                "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                instance_id,
                project_env_id
            )
            .execute(&state.pool)
            .await?;
            crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, instance_id);
        }
    }

    let pool_clone = state.pool.clone();
    let container_name_clone = container_name.clone();
    let image_for_container = full_image.clone();
    let type_enum_clone = payload.r#type.clone();
    let db_user_clone = db_user.clone();
    let db_name_clone = db_name.clone();
    let ws_id_str = ws_id.to_string();
    let cpu_limit = payload.cpu_limit.unwrap_or(0);
    let memory_limit_mb = payload.memory_limit_mb.unwrap_or(0);
    let storage_gb = payload.storage_size_gb.filter(|&s| s > 0).unwrap_or(1);

    tokio::spawn(async move {
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(db_id = %db_id, "Failed to connect to K8s for database provisioning: {}", e);
                let _ = update_db_status(&pool_clone, db_id, DbStatus::Failed).await;
                return;
            }
        };

        let namespace = format!("hermes-ws-{}", ws_id_str);
        let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb, max_cpu_millicores FROM workspaces WHERE id = $1", ws_id)
            .fetch_one(&pool_clone)
            .await;
        let (max_mem, max_storage, max_cpu) = match limits {
            Ok(r) => (r.max_memory_mb, r.max_storage_gb, r.max_cpu_millicores),
            Err(_) => (2048, 10, 0),
        };
        if let Err(e) = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage, max_cpu).await {
            tracing::warn!(db_id = %db_id, namespace = %namespace, "create_namespace warning: {}", e);
            // Non-fatal: namespace may already exist, continue
        }

        let mut envs = Vec::new();
        match type_enum_clone {
            DbType::Postgres => {
                envs.push(("POSTGRES_USER".to_string(), db_user_clone));
                envs.push(("POSTGRES_PASSWORD".to_string(), raw_password));
                envs.push(("POSTGRES_DB".to_string(), db_name_clone));
            },
            DbType::Mysql => {
                envs.push(("MYSQL_ROOT_PASSWORD".to_string(), raw_password.clone()));
                envs.push(("MYSQL_USER".to_string(), db_user_clone));
                envs.push(("MYSQL_PASSWORD".to_string(), raw_password));
                envs.push(("MYSQL_DATABASE".to_string(), db_name_clone));
            },
            DbType::Mongodb => {
                envs.push(("MONGO_INITDB_ROOT_USERNAME".to_string(), db_user_clone));
                envs.push(("MONGO_INITDB_ROOT_PASSWORD".to_string(), raw_password));
            },
            DbType::Redis => {
                // Redis takes its password via env; deploy_database wires it into
                // `redis-server --requirepass "$REDIS_PASSWORD"` (empty = authless).
                envs.push(("REDIS_PASSWORD".to_string(), raw_password));
            }
        }

        match crate::utils::k8s::K8sManager::deploy_database(
            &k8s_client,
            &namespace,
            &container_name_clone,
            &image_for_container,
            envs,
            internal_port,
            cpu_limit,
            memory_limit_mb,
            storage_gb,
        ).await {
            Ok(_) => {
                if is_external {
                    let lb_name = format!("{}-external", container_name_clone);
                    let _ = crate::utils::k8s::K8sManager::deploy_loadbalancer_service(
                        &k8s_client,
                        &namespace,
                        &lb_name,
                        &container_name_clone,
                        internal_port,
                        ext_port.unwrap_or(internal_port),
                        "TCP",
                    ).await;
                }
                let _ = update_db_status(&pool_clone, db_id, DbStatus::Running).await;
            }
            Err(e) => {
                tracing::error!(
                    db_id = %db_id,
                    namespace = %namespace,
                    container = %container_name_clone,
                    "deploy_database failed (possibly quota exceeded during concurrent build): {}",
                    e
                );
                let _ = update_db_status(&pool_clone, db_id, DbStatus::Failed).await;
            }
        }
    });

    Ok((
        StatusCode::CREATED,
        Json(DatabaseResponse {
            id: db_id,
            project_id: payload.project_id,
            app_instance_id: payload.app_instance_id,
            name: payload.name,
            r#type: payload.r#type,
            version: full_image,
            db_user,
            db_name,
            container_name,
            internal_port,
            is_external,
            external_port: ext_port,
            status: DbStatus::Provisioning,
            connection_url,
        }),
    ))
}

pub async fn get_database(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<Json<DatabaseService>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let mut db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    // Dynamically check and sync status with actual K8s pod state
    if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
        let namespace = format!("hermes-ws-{}", ws_id);
        let actual_status = get_actual_db_status(&k8s_client, &namespace, &db_service.container_name, db_service.status.clone()).await;
        if actual_status != db_service.status {
            db_service.status = actual_status.clone();
            let _ = update_db_status(&state.pool, db_service.id, actual_status).await;
        }
    }

    Ok(Json(db_service))
}

pub async fn delete_database(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = db_service.container_name.clone();

    tokio::spawn(async move {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let _ = crate::utils::k8s::K8sManager::delete_database(&k8s_client, &namespace, &container_name).await;
        }
    });

    // Remove the project-pool var this database published; the link cascade
    // detaches it from every app. Hot-reload the running ones that used it.
    let linked: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT ael.app_instance_id FROM app_env_links ael
         JOIN project_env_variables pev ON pev.id = ael.project_env_id
         WHERE pev.source = 'database' AND pev.source_id = $1",
        db_id
    )
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    sqlx::query!(
        "DELETE FROM project_env_variables WHERE source = 'database' AND source_id = $1",
        db_id
    )
    .execute(&state.pool)
    .await?;
    for instance_id in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, instance_id);
    }

    // Remove any cron jobs (incl. the managed backup) targeting this database.
    let _ = sqlx::query!("DELETE FROM cron_jobs WHERE target_type = 'database' AND target_id = $1", db_id)
        .execute(&state.pool).await;

    sqlx::query!("DELETE FROM databases WHERE id = $1 AND workspace_id = $2", db_id, ws_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// Provision a database container in Kubernetes (namespace, deploy, optional
/// external LoadBalancer) and update its status. Reusable by create_database and
/// the docker-compose auto-split applier.
pub(crate) fn spawn_db_provisioning(
    pool: sqlx::PgPool,
    ws_id: Uuid,
    db_id: Uuid,
    db_type: DbType,
    container_name: String,
    image: String,
    db_user: String,
    raw_password: String,
    db_name: String,
    internal_port: i32,
    cpu_limit: i32,
    memory_limit_mb: i64,
    is_external: bool,
    ext_port: Option<i32>,
) {
    tokio::spawn(async move {
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(db_id = %db_id, "Failed to connect to K8s for database provisioning: {}", e);
                let _ = update_db_status(&pool, db_id, DbStatus::Failed).await;
                return;
            }
        };

        let namespace = format!("hermes-ws-{}", ws_id);
        let (max_mem, max_storage, max_cpu) = match sqlx::query!("SELECT max_memory_mb, max_storage_gb, max_cpu_millicores FROM workspaces WHERE id = $1", ws_id).fetch_one(&pool).await {
            Ok(r) => (r.max_memory_mb, r.max_storage_gb, r.max_cpu_millicores),
            Err(_) => (2048, 10, 0),
        };
        let _ = crate::utils::k8s::K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage, max_cpu).await;

        let mut envs = Vec::new();
        match db_type {
            DbType::Postgres => {
                envs.push(("POSTGRES_USER".to_string(), db_user));
                envs.push(("POSTGRES_PASSWORD".to_string(), raw_password));
                envs.push(("POSTGRES_DB".to_string(), db_name));
            }
            DbType::Mysql => {
                envs.push(("MYSQL_ROOT_PASSWORD".to_string(), raw_password.clone()));
                envs.push(("MYSQL_USER".to_string(), db_user));
                envs.push(("MYSQL_PASSWORD".to_string(), raw_password));
                envs.push(("MYSQL_DATABASE".to_string(), db_name));
            }
            DbType::Mongodb => {
                envs.push(("MONGO_INITDB_ROOT_USERNAME".to_string(), db_user));
                envs.push(("MONGO_INITDB_ROOT_PASSWORD".to_string(), raw_password));
            }
            DbType::Redis => {
                envs.push(("REDIS_PASSWORD".to_string(), raw_password));
            }
        }

        let storage_gb = sqlx::query_scalar!("SELECT storage_size_gb FROM databases WHERE id = $1", db_id)
            .fetch_one(&pool).await.unwrap_or(1);
        match crate::utils::k8s::K8sManager::deploy_database(
            &k8s_client, &namespace, &container_name, &image, envs, internal_port, cpu_limit, memory_limit_mb, storage_gb,
        ).await {
            Ok(_) => {
                if is_external {
                    let lb_name = format!("{}-external", container_name);
                    let _ = crate::utils::k8s::K8sManager::deploy_loadbalancer_service(
                        &k8s_client, &namespace, &lb_name, &container_name, internal_port, ext_port.unwrap_or(internal_port), "TCP",
                    ).await;
                }
                
                // Wait until the container is actually running and ready in Kubernetes.
                let mut ready = false;
                for _ in 0..150 { // 150 * 2 seconds = 5 minutes timeout
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let actual = get_actual_db_status(&k8s_client, &namespace, &container_name, DbStatus::Provisioning).await;
                    if actual == DbStatus::Running {
                        ready = true;
                        break;
                    } else if actual == DbStatus::Failed {
                        break;
                    }
                }
                
                let final_status = if ready { DbStatus::Running } else { DbStatus::Failed };
                let _ = update_db_status(&pool, db_id, final_status).await;
            }
            Err(e) => {
                tracing::error!(db_id = %db_id, "deploy_database failed: {}", e);
                let _ = update_db_status(&pool, db_id, DbStatus::Failed).await;
            }
        }
    });
}

async fn update_db_status(pool: &sqlx::PgPool, id: Uuid, status: DbStatus) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE databases SET status = $1, updated_at = now() WHERE id = $2", status.clone() as DbStatus, id)
        .execute(pool)
        .await?;

    if let Ok(Some(meta)) = sqlx::query!(
        "SELECT workspace_id, container_name FROM databases WHERE id = $1",
        id
    )
    .fetch_optional(pool)
    .await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::DatabaseStatusChanged {
                workspace_id: meta.workspace_id,
                database_id: id,
                container_name: meta.container_name,
                status: format!("{:?}", status).to_lowercase(),
            }
        );
    }

    Ok(())
}

pub async fn reveal_database_credentials(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let decrypted_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => {
            crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?
        }
    };

    let raw_url = match db_service.r#type {
        DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Redis => format!("redis://:{}@{}:{}", decrypted_password, db_service.container_name, db_service.internal_port),
        DbType::Mongodb => format!("mongodb://{}:{}@{}:{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port),
    };

    Ok(Json(serde_json::json!({
        "databaseUser": db_service.db_user,
        "databasePassword": decrypted_password,
        "connectionUrl": raw_url,
    })))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseListQuery {
    pub project_id: Uuid,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

pub async fn list_project_databases(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(query): Query<DatabaseListQuery>,
) -> Result<Json<crate::utils::pagination::Paginated<DatabaseService>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let (page, page_size, offset) = crate::utils::pagination::PaginationParams {
        page: query.page,
        page_size: query.page_size,
    }.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM databases WHERE project_id = $1 AND workspace_id = $2",
        query.project_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let mut databases = sqlx::query_as::<_, DatabaseService>(
        "SELECT * FROM databases WHERE project_id = $1 AND workspace_id = $2 ORDER BY created_at DESC LIMIT $3 OFFSET $4"
    )
    .bind(query.project_id)
    .bind(ws_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    // Dynamically check and sync status with actual K8s pod state
    if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
        let namespace = format!("hermes-ws-{}", ws_id);
        for db in &mut databases {
            let actual_status = get_actual_db_status(&k8s_client, &namespace, &db.container_name, db.status.clone()).await;
            if actual_status != db.status {
                db.status = actual_status.clone();
                let _ = update_db_status(&state.pool, db.id, actual_status).await;
            }
        }
    }

    Ok(Json(crate::utils::pagination::Paginated::new(databases, total, page, page_size)))
}

#[derive(Debug, serde::Deserialize)]
pub struct DatabaseQueryRequest {
    pub query: String,
}

#[derive(Debug, serde::Serialize)]
pub struct DatabaseQueryResponse {
    pub output: String,
    pub is_error: bool,
}

pub async fn execute_database_query(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
    Json(payload): Json<DatabaseQueryRequest>,
) -> Result<Json<DatabaseQueryResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let decrypted_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => {
            crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?
        }
    };

    let namespace = format!("hermes-ws-{}", ws_id);
    let pod_name = format!("{}-0", db_service.container_name);

    let mut cmd = std::process::Command::new("kubectl");
    cmd.arg("exec");
    cmd.arg("-n");
    cmd.arg(&namespace);
    cmd.arg(&pod_name);
    cmd.arg("--");

    match db_service.r#type {
        DbType::Postgres => {
            cmd.arg("psql");
            cmd.arg("-U");
            cmd.arg(&db_service.db_user);
            cmd.arg("-d");
            cmd.arg(&db_service.db_name);
            cmd.arg("-c");
            cmd.arg(&payload.query);
        }
        DbType::Mysql => {
            cmd.arg("mysql");
            cmd.arg("-u");
            cmd.arg(&db_service.db_user);
            cmd.arg(format!("-p{}", decrypted_password));
            cmd.arg("-e");
            cmd.arg(&payload.query);
            cmd.arg(&db_service.db_name);
        }
        DbType::Redis => {
            cmd.arg("redis-cli");
            cmd.arg("-a");
            cmd.arg(&decrypted_password);
            cmd.arg("--no-auth-warning");
            let parts: Vec<&str> = payload.query.split_whitespace().collect();
            for part in parts {
                cmd.arg(part);
            }
        }
        DbType::Mongodb => {
            cmd.arg("sh");
            cmd.arg("-c");
            let mongo_script = format!(
                "mongosh -u '{}' -p '{}' --authenticationDatabase admin --quiet --eval '{}' || mongo -u '{}' -p '{}' --authenticationDatabase admin --quiet --eval '{}'",
                db_service.db_user, decrypted_password, payload.query.replace('\'', "'\\''"),
                db_service.db_user, decrypted_password, payload.query.replace('\'', "'\\''")
            );
            cmd.arg(mongo_script);
        }
    }

    let output = match cmd.output() {
        Ok(out) => out,
        Err(e) => {
            return Ok(Json(DatabaseQueryResponse {
                output: format!("Failed to run kubectl command: {}", e),
                is_error: true,
            }));
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let is_error = !output.status.success();
    let final_output = if is_error {
        if stderr.is_empty() { stdout } else { stderr }
    } else {
        stdout
    };

    Ok(Json(DatabaseQueryResponse {
        output: final_output,
        is_error,
    }))
}

#[derive(Debug, serde::Deserialize)]
pub struct LogQuery {
    pub previous: Option<bool>,
}

pub async fn stream_database_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(query): Query<LogQuery>,
    Path(db_id): Path<Uuid>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = db_service.container_name.clone();
    let is_previous = query.previous.unwrap_or(false);

    let sse_stream = async_stream::stream! {
        let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client.clone(), &namespace);
        let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));

        loop {
            let pod_list = match pods_api.list(&lp).await {
                Ok(list) => list,
                Err(e) => {
                    yield Ok(Event::default().data(format!("[Console Error] Eșec la listarea pod-urilor din Kubernetes: {}", e)));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
            };

            let pod = match pod_list.items.first() {
                Some(p) => p,
                None => {
                    yield Ok(Event::default().data("[Console] Se așteaptă programarea pod-ului pe nod...".to_string()));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
            };

            let pod_name = match &pod.metadata.name {
                Some(name) => name.clone(),
                None => {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }
            };

            let phase = pod.status.as_ref()
                .and_then(|s| s.phase.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            if phase == "Pending" || phase == "Unknown" {
                yield Ok(Event::default().data(format!("[Console] Baza de date se inițializează (Stare: {})...", phase)));
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }

            let log_params = kube::api::LogParams {
                follow: !is_previous && (phase == "Running"),
                previous: is_previous,
                tail_lines: Some(100),
                ..Default::default()
            };

            let log_stream_res = pods_api.log_stream(&pod_name, &log_params).await;
            match log_stream_res {
                Ok(log_stream) => {
                    yield Ok(Event::default().data("[Console] Conexiune stabilă cu pod-ul. Se preiau logurile:".to_string()));
                    
                    use futures_util::io::AsyncBufReadExt;
                    let mut lines = log_stream.lines();
                    while let Some(line_res) = lines.next().await {
                        match line_res {
                            Ok(line) => {
                                yield Ok(Event::default().data(line));
                            }
                            Err(e) => {
                                yield Ok(Event::default().data(format!("[Console Warning] Eroare de rețea la fluxul de logs: {}", e)));
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    yield Ok(Event::default().data(format!("[Console] Se pornește containerul de logs (Eroare API: {})...", e)));
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    };

    Ok(Sse::new(sse_stream))
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDatabaseSettingsRequest {
    pub cpu_limit: i32,
    pub memory_limit_mb: i64,
    pub backup_enabled: Option<bool>,
    pub backup_count: Option<i32>,
}

pub async fn update_database_settings(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
    Json(payload): Json<UpdateDatabaseSettingsRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // 1. Fetch the database service metadata (including container name and type)
    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    // Check workspace memory limits
    crate::utils::limits::check_workspace_memory_limit(
        &state.pool,
        ws_id,
        payload.memory_limit_mb,
        Some(db_id)
    ).await?;
    if payload.cpu_limit > 0 {
        crate::utils::limits::check_workspace_cpu_limit(&state.pool, ws_id, payload.cpu_limit, Some(db_id)).await?;
    }

    // 2. Update resource limits and backup settings in databases table
    let backup_enabled = payload.backup_enabled.unwrap_or(false);
    let backup_count = payload.backup_count.unwrap_or(7);
    sqlx::query!(
        "UPDATE databases SET cpu_limit = $1, memory_limit_mb = $2, backup_enabled = $3, backup_count = $4, updated_at = now() WHERE id = $5",
        payload.cpu_limit, payload.memory_limit_mb, backup_enabled, backup_count, db_id
    )
    .execute(&state.pool)
    .await?;

    // Materialize auto-backup as a real, visible, editable cron job (or remove it).
    if backup_enabled {
        let _ = crate::controllers::cron_controller::ensure_backup_cron(&state.pool, db_id).await;
    } else {
        let _ = crate::controllers::cron_controller::remove_backup_cron(&state.pool, db_id).await;
    }

    // 3. Decrypt the database password
    let decrypted_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => {
            crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?
        }
    };

    // 4. Update/Upsert the environment variable DATABASE_URL with the new container name & credentials
    let new_connection_url = match db_service.r#type {
        DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Redis => format!("redis://:{}@{}:{}", decrypted_password, db_service.container_name, db_service.internal_port),
        DbType::Mongodb => format!("mongodb://{}:{}@{}:{}", db_service.db_user, decrypted_password, db_service.container_name, db_service.internal_port),
    };

    if let Some(instance_id) = db_service.app_instance_id {
        let (enc_url, nonce_url) = crypto::encrypt_env_value(&new_connection_url)?;
        sqlx::query!(
            "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
             VALUES ($1, $2, $3, 'DATABASE_URL', $4, $5, true)
             ON CONFLICT (app_instance_id, key) DO UPDATE SET encrypted_value = $4, nonce = $5",
            Uuid::new_v4(), ws_id, instance_id, enc_url, nonce_url
        )
        .execute(&state.pool)
        .await?;
    }

    // 5. Build K8s configuration and deploy database
    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);

    let mut envs = Vec::new();
    match db_service.r#type {
        DbType::Postgres => {
            envs.push(("POSTGRES_USER".to_string(), db_service.db_user.clone()));
            envs.push(("POSTGRES_PASSWORD".to_string(), decrypted_password));
            envs.push(("POSTGRES_DB".to_string(), db_service.db_name.clone()));
        },
        DbType::Mysql => {
            envs.push(("MYSQL_ROOT_PASSWORD".to_string(), decrypted_password.clone()));
            envs.push(("MYSQL_USER".to_string(), db_service.db_user.clone()));
            envs.push(("MYSQL_PASSWORD".to_string(), decrypted_password));
            envs.push(("MYSQL_DATABASE".to_string(), db_service.db_name.clone()));
        },
        DbType::Mongodb => {
            envs.push(("MONGO_INITDB_ROOT_USERNAME".to_string(), db_service.db_user.clone()));
            envs.push(("MONGO_INITDB_ROOT_PASSWORD".to_string(), decrypted_password));
        },
        DbType::Redis => {
            envs.push(("REDIS_PASSWORD".to_string(), decrypted_password));
        }
    }

    // Update database status based on deployment success
    match crate::utils::k8s::K8sManager::deploy_database(
        &k8s_client,
        &namespace,
        &db_service.container_name,
        &db_service.version,
        envs,
        db_service.internal_port,
        payload.cpu_limit,
        payload.memory_limit_mb,
        db_service.storage_size_gb,
    ).await {
        Ok(_) => {
            let _ = update_db_status(&state.pool, db_id, DbStatus::Running).await;
        }
        Err(e) => {
            let _ = update_db_status(&state.pool, db_id, DbStatus::Failed).await;
            return Err(e);
        }
    }

    Ok(StatusCode::OK)
}

async fn get_actual_db_status(
    k8s_client: &kube::Client,
    namespace: &str,
    container_name: &str,
    db_status_in_db: DbStatus,
) -> DbStatus {
    if db_status_in_db == DbStatus::Failed || db_status_in_db == DbStatus::Stopped {
        return db_status_in_db;
    }

    let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client.clone(), namespace);
    let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));

    match pods_api.list(&lp).await {
        Ok(list) => {
            if let Some(pod) = list.items.first() {
                if let Some(status) = &pod.status {
                    let phase = status.phase.as_deref().unwrap_or("Unknown");
                    
                    let container_ready = status.container_statuses.as_ref()
                        .and_then(|statuses| statuses.first())
                        .map(|c_status| c_status.ready)
                        .unwrap_or(false);

                    if phase == "Running" && container_ready {
                        DbStatus::Running
                    } else if phase == "Failed" {
                        DbStatus::Failed
                    } else {
                        // Pending, ContainerCreating, or Running but container not yet ready (initializing)
                        DbStatus::Provisioning
                    }
                } else {
                    DbStatus::Provisioning
                }
            } else {
                // Pod does not exist yet (meaning it is still scheduling or creating)
                DbStatus::Provisioning
            }
        }
        Err(_) => db_status_in_db,
    }
}

pub async fn perform_database_backup(pool: &sqlx::PgPool, db_id: Uuid, custom_command: Option<&str>) -> Result<BackupResponse, AppError> {
    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1")
        .bind(db_id).fetch_optional(pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let decrypted_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?
    };

    let namespace = format!("hermes-ws-{}", db_service.workspace_id);
    let pod_name = format!("{}-0", db_service.container_name);

    let extension = match db_service.r#type {
        DbType::Postgres | DbType::Mysql => "sql",
        DbType::Mongodb => "archive",
        DbType::Redis => "rdb",
    };
    
    let backups_dir = format!("/var/lib/hermes/backups/{}", db_id);
    std::fs::create_dir_all(&backups_dir).map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to create backups directory: {}", e)))?;
    
    let filename = format!("{}.{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"), extension);
    let filepath = format!("{}/{}", backups_dir, filename);
    let file = std::fs::File::create(&filepath).map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to create backup file on host: {}", e)))?;

    let mut cmd = std::process::Command::new("kubectl");
    cmd.arg("exec");
    cmd.arg("-n");
    cmd.arg(&namespace);
    cmd.arg(&pod_name);
    cmd.arg("--");

    // A custom command (from the editable backup cron) runs as a shell snippet inside
    // the DB pod; otherwise fall back to the per-type default dump.
    if let Some(custom) = custom_command.map(str::trim).filter(|s| !s.is_empty()) {
        cmd.arg("/bin/sh");
        cmd.arg("-c");
        cmd.arg(custom);
    } else {
        match db_service.r#type {
            DbType::Postgres => {
                cmd.arg("pg_dump");
                // --clean --if-exists make the dump drop each object before recreating it,
                // so a restore replaces existing data instead of appending duplicate rows.
                cmd.arg("--clean");
                cmd.arg("--if-exists");
                cmd.arg("-U");
                cmd.arg(&db_service.db_user);
                cmd.arg("-d");
                cmd.arg(&db_service.db_name);
            }
            DbType::Mysql => {
                cmd.arg("mysqldump");
                // --add-drop-table (default on) emits DROP TABLE before each CREATE,
                // making restores replace rather than append; set explicitly to be safe.
                cmd.arg("--add-drop-table");
                cmd.arg("-u");
                cmd.arg(&db_service.db_user);
                cmd.arg(format!("-p{}", decrypted_password));
                cmd.arg(&db_service.db_name);
            }
            DbType::Mongodb => {
                cmd.arg("mongodump");
                cmd.arg("--username");
                cmd.arg(&db_service.db_user);
                cmd.arg("--password");
                cmd.arg(&decrypted_password);
                cmd.arg("--authenticationDatabase");
                cmd.arg("admin");
                cmd.arg("--archive");
            }
            DbType::Redis => {
                cmd.arg("redis-cli");
                cmd.arg("-a");
                cmd.arg(&decrypted_password);
                cmd.arg("--no-auth-warning");
                cmd.arg("--rdb");
                cmd.arg("-");
            }
        }
    }

    cmd.stdout(std::process::Stdio::from(file));
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to spawn backup process: {}", e)))?;
    let output = child.wait_with_output().map_err(|e| AppError::Fatal(anyhow::anyhow!("Backup process execution failed: {}", e)))?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
        let _ = std::fs::remove_file(&filepath);
        return Err(AppError::Fatal(anyhow::anyhow!("Backup command failed: {}", err_msg)));
    }

    let file_size = std::fs::metadata(&filepath).map(|m| m.len() as i64).unwrap_or(0);
    let backup_id = Uuid::new_v4();
    let created_at = chrono::Utc::now();

    sqlx::query!(
        "INSERT INTO database_backups (id, database_id, filename, file_size_bytes, status, created_at)
         VALUES ($1, $2, $3, $4, 'completed', $5)",
        backup_id, db_id, filename, file_size, created_at
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        "UPDATE databases SET last_backup_at = $1 WHERE id = $2",
        created_at, db_id
    )
    .execute(pool)
    .await?;

    let backup_limit = db_service.backup_count;
    let old_backups = sqlx::query!(
        "SELECT id, filename FROM database_backups WHERE database_id = $1 ORDER BY created_at DESC OFFSET $2",
        db_id, backup_limit as i64
    )
    .fetch_all(pool)
    .await;

    if let Ok(backups_to_delete) = old_backups {
        for old_b in backups_to_delete {
            let old_filepath = format!("{}/{}", backups_dir, old_b.filename);
            let _ = std::fs::remove_file(&old_filepath);
            let _ = sqlx::query!(
                "DELETE FROM database_backups WHERE id = $1",
                old_b.id
            )
            .execute(pool)
            .await;
        }
    }

    // Surface any stderr (e.g. the command's friendly echo) into the cron history.
    let stderr_msg = String::from_utf8_lossy(&output.stderr).trim().to_string();

    Ok(BackupResponse {
        id: backup_id,
        database_id: db_id,
        filename,
        file_size_bytes: file_size,
        status: "completed".to_string(),
        created_at,
        log: if stderr_msg.is_empty() { None } else { Some(stderr_msg) },
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupCronResponse {
    pub id: Uuid,
    pub name: String,
    pub schedule: String,
    pub command: String,
    pub status: String,
    pub next_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// The managed backup cron for a database (or null if auto-backup is off).
pub async fn get_database_backup_cron(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<Json<Option<BackupCronResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let row = sqlx::query!(
        "SELECT id, name, schedule, command, status::text as status, next_run_at
         FROM cron_jobs
         WHERE target_type = 'database' AND target_id = $1 AND is_backup = true AND workspace_id = $2
         LIMIT 1",
        db_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?;

    Ok(Json(row.map(|r| BackupCronResponse {
        id: r.id,
        name: r.name,
        schedule: r.schedule,
        command: r.command,
        status: r.status.unwrap_or_default(),
        next_run_at: r.next_run_at,
    })))
}

pub async fn create_database_backup(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<(StatusCode, Json<BackupResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let _ = sqlx::query!("SELECT id FROM databases WHERE id = $1 AND workspace_id = $2", db_id, ws_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let backup_res = perform_database_backup(&state.pool, db_id, None).await?;

    Ok((
        StatusCode::CREATED,
        Json(backup_res),
    ))
}

pub async fn list_database_backups(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<Json<Vec<BackupResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify DB exists in workspace
    let db_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM databases WHERE id = $1 AND workspace_id = $2)",
        db_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !db_exists {
        return Err(AppError::NotFound("Database not found in this workspace.".to_string()));
    }

    let records = sqlx::query!(
        "SELECT id, database_id, filename, file_size_bytes, status, created_at
         FROM database_backups
         WHERE database_id = $1
         ORDER BY created_at DESC",
        db_id
    )
    .fetch_all(&state.pool)
    .await?;

    let response = records
        .into_iter()
        .map(|r| BackupResponse {
            id: r.id,
            database_id: r.database_id,
            filename: r.filename,
            file_size_bytes: r.file_size_bytes,
            status: r.status,
            created_at: r.created_at,
            log: None,
        })
        .collect();

    Ok(Json(response))
}

pub async fn delete_database_backup(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((db_id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify DB exists in workspace
    let db_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM databases WHERE id = $1 AND workspace_id = $2)",
        db_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !db_exists {
        return Err(AppError::NotFound("Database not found in this workspace.".to_string()));
    }

    let backup = sqlx::query!(
        "SELECT filename FROM database_backups WHERE id = $1 AND database_id = $2",
        backup_id, db_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Backup not found for this database.".to_string()))?;

    let filepath = format!("/var/lib/hermes/backups/{}/{}", db_id, backup.filename);
    let _ = std::fs::remove_file(filepath);

    sqlx::query!("DELETE FROM database_backups WHERE id = $1", backup_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn restore_database_backup(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((db_id, backup_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    let backup = sqlx::query!(
        "SELECT filename FROM database_backups WHERE id = $1 AND database_id = $2",
        backup_id, db_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Backup not found for this database.".to_string()))?;

    let filepath = format!("/var/lib/hermes/backups/{}/{}", db_id, backup.filename);
    if !std::path::Path::new(&filepath).exists() {
        return Err(AppError::NotFound("Backup file not found physically on host disk.".to_string()));
    }

    let decrypted_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?
    };

    let namespace = format!("hermes-ws-{}", ws_id);
    let pod_name = format!("{}-0", db_service.container_name);

    if db_service.r#type == DbType::Redis {
        // Special Restore logic for Redis
        // Copy the backup file to a local relative path first to avoid Windows kubectl cp absolute path colon issues.
        let temp_filename = format!("temp_restore_{}.rdb", db_id);
        let temp_filepath = std::path::Path::new(&temp_filename);
        std::fs::copy(&filepath, &temp_filepath).map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to copy backup to temp restore file: {}", e)))?;

        let mut cp_cmd = std::process::Command::new("kubectl");
        cp_cmd.arg("cp");
        cp_cmd.arg(&temp_filename);
        cp_cmd.arg(format!("{}/{}:/data/dump.rdb", namespace, pod_name));
        
        let cp_output = cp_cmd.output().map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to run kubectl cp: {}", e)))?;
        
        // Clean up temporary file
        let _ = std::fs::remove_file(&temp_filepath);

        let stderr_msg = String::from_utf8_lossy(&cp_output.stderr).to_string();
        let stdout_msg = String::from_utf8_lossy(&cp_output.stdout).to_string();
        if !cp_output.status.success() || stderr_msg.to_lowercase().contains("error") {
            return Err(AppError::Fatal(anyhow::anyhow!("Failed to copy RDB backup file into Redis pod: status={:?}, stderr={}, stdout={}", cp_output.status, stderr_msg, stdout_msg)));
        }

        // Force shutdown Redis immediately without saving so it doesn't overwrite the copied dump.rdb.
        // The container will exit and Kubernetes will automatically restart it, loading the restored file.
        let mut shutdown_cmd = std::process::Command::new("kubectl");
        shutdown_cmd.arg("exec");
        shutdown_cmd.arg("-n");
        shutdown_cmd.arg(&namespace);
        shutdown_cmd.arg(&pod_name);
        shutdown_cmd.arg("--");
        shutdown_cmd.arg("redis-cli");
        shutdown_cmd.arg("shutdown");
        shutdown_cmd.arg("nosave");
        let _ = shutdown_cmd.status();
    } else {
        // SQL and MongoDB stdin restore
        let file = std::fs::File::open(&filepath).map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to open backup file for restore: {}", e)))?;
        
        let mut cmd = std::process::Command::new("kubectl");
        cmd.arg("exec");
        cmd.arg("-i");
        cmd.arg("-n");
        cmd.arg(&namespace);
        cmd.arg(&pod_name);
        cmd.arg("--");

        match db_service.r#type {
            DbType::Postgres => {
                // Drop and recreate schema public cascade to get a clean slate
                let mut prep_cmd = std::process::Command::new("kubectl");
                prep_cmd.args(&["exec", "-n", &namespace, &pod_name, "--", "psql", "-U", &db_service.db_user, "-d", &db_service.db_name, "-c", "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"]);
                let _ = prep_cmd.status(); // Ignore failure, just try our best to clean
                
                cmd.arg("psql");
                cmd.arg("-U");
                cmd.arg(&db_service.db_user);
                cmd.arg("-d");
                cmd.arg(&db_service.db_name);
            }
            DbType::Mysql => {
                // Drop and recreate database to get a clean slate
                let mut prep_cmd = std::process::Command::new("kubectl");
                prep_cmd.args(&["exec", "-n", &namespace, &pod_name, "--", "mysql", "-u", &db_service.db_user, &format!("-p{}", decrypted_password), "-e", &format!("DROP DATABASE IF EXISTS {}; CREATE DATABASE {};", db_service.db_name, db_service.db_name)]);
                let _ = prep_cmd.status(); // Ignore failure, just try our best to clean
                
                cmd.arg("mysql");
                cmd.arg("-u");
                cmd.arg(&db_service.db_user);
                cmd.arg(format!("-p{}", decrypted_password));
                cmd.arg(&db_service.db_name);
            }
            DbType::Mongodb => {
                cmd.arg("mongorestore");
                cmd.arg("--username");
                cmd.arg(&db_service.db_user);
                cmd.arg("--password");
                cmd.arg(&decrypted_password);
                cmd.arg("--authenticationDatabase");
                cmd.arg("admin");
                cmd.arg("--drop");
                cmd.arg("--archive");
            }
            DbType::Redis => unreachable!(),
        }

        cmd.stdin(std::process::Stdio::from(file));
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to spawn restore process: {}", e)))?;
        let output = child.wait_with_output().map_err(|e| AppError::Fatal(anyhow::anyhow!("Restore process execution failed: {}", e)))?;

        if !output.status.success() {
             let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
             return Err(AppError::Fatal(anyhow::anyhow!("Restore command failed: {}", err_msg)));
         }
     }
 
     Ok(StatusCode::OK)
 }
 
 #[derive(Debug, serde::Deserialize)]
 #[serde(rename_all = "camelCase")]
 pub struct DatabaseMetricsQuery {
     pub metric: String,
     pub range: Option<String>,
 }
 
 pub async fn get_database_metrics(
     State(state): State<AppState>,
     AuthenticatedUser(claims): AuthenticatedUser,
     Query(query): Query<DatabaseMetricsQuery>,
     Path(db_id): Path<Uuid>,
 ) -> Result<Json<crate::dtos::metrics_dto::MetricsHistoryResponse>, AppError> {
     let ws_id = claims.current_workspace_id.ok_or_else(|| {
         AppError::Validation("No active workspace selected.".to_string())
     })?;
 
     let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
         .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
         .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;
 
     let namespace = format!("hermes-ws-{}", ws_id);
     let container_name = db_service.container_name.clone();
     let range = query.range.unwrap_or_else(|| "1h".to_string());
 
     let engine = match db_service.r#type {
         crate::models::database_model::DbType::Postgres => "postgres",
         crate::models::database_model::DbType::Mysql => "mysql",
         crate::models::database_model::DbType::Redis => "redis",
         crate::models::database_model::DbType::Mongodb => "mongodb",
     };

     let (timestamps, values, simulated) = crate::utils::prometheus::get_historical_metrics(
         &namespace,
         &container_name,
         &query.metric,
         &range,
         engine,
     ).await?;

     Ok(Json(crate::dtos::metrics_dto::MetricsHistoryResponse {
         timestamps,
         values,
         simulated,
     }))
 }

/// POST /databases/:id/rotate-password — rotate the engine's password, persist the
/// new encrypted value, refresh the published DATABASE_URL and auto-reload the
/// consuming app instance so it reconnects.
///
/// Postgres/MySQL/Mongo: live ALTER USER inside the running pod (which must succeed
/// BEFORE we persist, so the stored value never diverges from the engine). Redis:
/// the password is a server arg, so we redeploy with the new REDIS_PASSWORD and
/// restart the pod (the first rotation also migrates an authless Redis to auth).
pub async fn rotate_database_password(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(db_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let db_service = sqlx::query_as::<_, DatabaseService>("SELECT * FROM databases WHERE id = $1 AND workspace_id = $2")
        .bind(db_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Database service not found.".to_string()))?;

    // Old password — needed to authenticate the ALTER for mysql/mongo.
    let old_password = match db_service.db_password_nonce {
        Some(ref nonce) => crypto::decrypt_env_value(&db_service.db_password, nonce)?,
        None => crypto::decrypt_env_value(&db_service.db_password, "AAAAAAAAAAAAAAAA")?,
    };

    let new_password = crate::utils::string_gen::generate_secure_string(32);

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let pod = crate::utils::k8s::K8sManager::pod_name_for_app(&k8s_client, &namespace, &db_service.container_name).await?;

    if matches!(db_service.r#type, DbType::Redis) {
        // Redis' password is config (a server arg), not stored data — so an in-place
        // ALTER doesn't apply. Persist it, redeploy so the Secret carries the new
        // REDIS_PASSWORD, then restart the pod so redis-server boots with the new
        // requirepass. (First rotation also migrates an authless Redis to auth.)
        let (enc, nonce) = crypto::encrypt_env_value(&new_password)?;
        sqlx::query!(
            "UPDATE databases SET db_password = $1, db_password_nonce = $2, updated_at = now() WHERE id = $3",
            enc, nonce, db_id
        )
        .execute(&state.pool)
        .await?;

        crate::utils::k8s::K8sManager::deploy_database(
            &k8s_client,
            &namespace,
            &db_service.container_name,
            &db_service.version,
            vec![("REDIS_PASSWORD".to_string(), new_password.clone())],
            db_service.internal_port,
            db_service.cpu_limit,
            db_service.memory_limit_mb,
            db_service.storage_size_gb,
        )
        .await?;
        crate::utils::k8s::K8sManager::delete_pod(&k8s_client, &namespace, &pod).await?;
    } else {
        let command: Vec<String> = match db_service.r#type {
            DbType::Postgres => vec![
                "psql".into(), "-U".into(), db_service.db_user.clone(), "-d".into(), db_service.db_name.clone(),
                "-c".into(), format!("ALTER USER \"{}\" WITH PASSWORD '{}';", db_service.db_user, new_password),
            ],
            DbType::Mysql => vec![
                "mysql".into(), "-uroot".into(), format!("-p{}", old_password), "-e".into(),
                format!(
                    "ALTER USER '{}'@'%' IDENTIFIED BY '{}'; ALTER USER 'root'@'%' IDENTIFIED BY '{}'; FLUSH PRIVILEGES;",
                    db_service.db_user, new_password, new_password
                ),
            ],
            // The root user lives in the `admin` db, so target it explicitly via
            // getSiblingDB (the eval otherwise runs against the default `test` db).
            // mongo:6.0 (the Hermes default) ships `mongosh`.
            DbType::Mongodb => vec![
                "mongosh".into(), "-u".into(), db_service.db_user.clone(), "-p".into(), old_password.clone(),
                "--authenticationDatabase".into(), "admin".into(), "--quiet".into(),
                "--eval".into(),
                format!("db.getSiblingDB('admin').changeUserPassword('{}','{}')", db_service.db_user, new_password),
            ],
            DbType::Redis => unreachable!("redis handled above"),
        };

        // Live password change (errors out cleanly if the pod isn't reachable).
        crate::utils::k8s::K8sManager::exec_in_pod(&k8s_client, &namespace, &pod, command).await?;

        // Engine changed — now persist the new encrypted password.
        let (enc, nonce) = crypto::encrypt_env_value(&new_password)?;
        sqlx::query!(
            "UPDATE databases SET db_password = $1, db_password_nonce = $2, updated_at = now() WHERE id = $3",
            enc, nonce, db_id
        )
        .execute(&state.pool)
        .await?;
    }

    // Refresh the published DATABASE_URL for the consuming instance, then reload it.
    let new_connection_url = match db_service.r#type {
        DbType::Postgres => format!("postgresql://{}:{}@{}:{}/{}", db_service.db_user, new_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Mysql => format!("mysql://{}:{}@{}:{}/{}", db_service.db_user, new_password, db_service.container_name, db_service.internal_port, db_service.db_name),
        DbType::Mongodb => format!("mongodb://{}:{}@{}:{}", db_service.db_user, new_password, db_service.container_name, db_service.internal_port),
        DbType::Redis => format!("redis://:{}@{}:{}", new_password, db_service.container_name, db_service.internal_port),
    };

    if let Some(instance_id) = db_service.app_instance_id {
        let (enc_url, nonce_url) = crypto::encrypt_env_value(&new_connection_url)?;
        sqlx::query!(
            "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
             VALUES ($1, $2, $3, 'DATABASE_URL', $4, $5, true)
             ON CONFLICT (app_instance_id, key) DO UPDATE SET encrypted_value = $4, nonce = $5",
            Uuid::new_v4(), ws_id, instance_id, enc_url, nonce_url
        )
        .execute(&state.pool)
        .await?;

        // Auto-reload the consumer so it reconnects with the new credentials.
        let pool_clone = state.pool.clone();
        tokio::spawn(async move {
            let tag = crate::utils::builder::resolve_instance_image_tag(&pool_clone, instance_id).await;
            crate::utils::builder::deploy_compiled_app(pool_clone, instance_id, tag).await;
        });
    }

    tracing::info!(db_id = %db_id, engine = ?db_service.r#type, reloaded_instance = ?db_service.app_instance_id, "Database password rotated");

    Ok(Json(serde_json::json!({
        "status": "rotated",
        "reloaded_instance": db_service.app_instance_id,
    })))
}