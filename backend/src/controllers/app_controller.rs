use axum::{
    extract::{State, Path, Query, ws::{Message, WebSocket, WebSocketUpgrade}},
    http::{StatusCode, HeaderMap},
    Json,
    response::{sse::{Event, Sse}, Response},
};
use uuid::Uuid;
use futures_util::stream::Stream;
use futures_util::stream::StreamExt;
use futures_util::SinkExt;
use std::convert::Infallible;
use serde::Deserialize;

use crate::app_state::AppState;
use crate::models::app_model::{App, AppInstance, AppInstanceType, AppStatus};
use crate::dtos::app_dto::{CreateAppRequest, CreateBranchRequest, AppDetailedResponse, AppInstanceResponse, ConfigureServerlessRequest};
use crate::dtos::build_dto::{BuildResponse, BuildDetailResponse, BuildQueueItem};
use crate::dtos::metrics_dto::MetricsHistoryResponse;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::error::AppError;
use crate::utils::pagination::{PaginationParams, Paginated};

#[derive(Debug, Deserialize)]
pub struct GitHubWebhookPayload {
    pub action: Option<String>,
    pub r#ref: Option<String>,
    pub ref_type: Option<String>,
    pub pull_request: Option<PRDetails>,
    pub repository: Option<RepoDetails>,
    pub head_commit: Option<CommitDetails>,
}

#[derive(Debug, Deserialize)]
pub struct CommitDetails {
    pub id: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstanceSettingsRequest {
    pub cpu_limit: Option<i32>,
    pub memory_limit_mb: Option<i64>,
    pub internal_port: Option<i32>,
    pub external_port: Option<i32>,
    pub build_command: Option<String>,
    pub start_command: Option<String>,
    pub replicas_min: Option<i32>,
    pub replicas_max: Option<i32>,
    pub autoscale_cpu_percent: Option<i32>,
    pub auto_sleep_enabled: Option<bool>,
    pub auto_sleep_after_minutes: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct PRDetails {
    pub merged: bool,
    pub head: PRHead,
}

#[derive(Debug, Deserialize)]
pub struct PRHead {
    pub r#ref: String,
}

#[derive(Debug, Deserialize)]
pub struct RepoDetails {
    pub ssh_url: Option<String>,
    pub clone_url: Option<String>,
}

pub async fn get_random_available_port(pool: &sqlx::PgPool) -> Result<i32, AppError> {
    for _ in 0..100 {
        let port: i32 = (rand::random::<u32>() % 10000 + 20000) as i32;
        let port_in_use_apps = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM app_instances WHERE external_port = $1)",
            port
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        let port_in_use_dbs = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM databases WHERE external_port = $1 AND is_external = true)",
            port
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if !port_in_use_apps && !port_in_use_dbs {
            return Ok(port);
        }
    }
    Err(AppError::Fatal(anyhow::anyhow!("Could not allocate a free external port after 100 attempts.")))
}

fn parse_github_repo(url: &str) -> Option<(String, String)> {
    let clean = url.trim().trim_end_matches(".git");
    if clean.contains("github.com") {
        if clean.starts_with("http") {
            let parts: Vec<&str> = clean.split("github.com/").collect();
            if parts.len() > 1 {
                let subparts: Vec<&str> = parts[1].split('/').collect();
                if subparts.len() >= 2 {
                    return Some((subparts[0].to_string(), subparts[1].to_string()));
                }
            }
        } else if clean.starts_with("git@") {
            let parts: Vec<&str> = clean.split(':').collect();
            if parts.len() > 1 {
                let subparts: Vec<&str> = parts[1].split('/').collect();
                if subparts.len() >= 2 {
                    return Some((subparts[0].to_string(), subparts[1].to_string()));
                }
            }
        }
    }
    None
}

/// Best-effort GitHub push-webhook registration for auto-deploy on push.
///
/// Resolves a token from the app's workspace git credential (the current system),
/// falling back to the legacy per-user `users.github_token`. Shared by the normal
/// create-app flow and the docker-compose split/import flows (which previously
/// never registered a webhook at all). Fire-and-forget: failures are logged.
pub async fn try_register_github_webhook(
    pool: &sqlx::PgPool,
    ws_id: Uuid,
    user_id: Uuid,
    git_credential_id: Option<Uuid>,
    git_repository: &str,
    host: &str,
) {
    if host.trim().is_empty() {
        return;
    }

    let mut gh_token: Option<String> = None;
    if let Some(cred_id) = git_credential_id {
        if let Ok(Some((enc, nonce))) = sqlx::query_as::<_, (String, String)>(
            "SELECT encrypted_token, nonce FROM git_credentials WHERE id = $1 AND workspace_id = $2",
        )
        .bind(cred_id)
        .bind(ws_id)
        .fetch_optional(pool)
        .await
        {
            gh_token = crate::utils::crypto::decrypt_env_value(&enc, &nonce).ok();
        }
    }
    if gh_token.is_none() {
        gh_token = sqlx::query!("SELECT github_token FROM users WHERE id = $1", user_id)
            .fetch_one(pool)
            .await
            .ok()
            .and_then(|r| r.github_token);
    }

    let (Some(token), Some((owner, repo))) = (gh_token, parse_github_repo(git_repository)) else {
        return;
    };

    let proto = if host.contains("localhost") || host.contains("127.0.0.1") || host.contains("192.168.") {
        "http"
    } else {
        "https"
    };
    let webhook_url = format!("{}://{}/api/v1/apps/webhook", proto, host);

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut config = serde_json::json!({
            "url": webhook_url,
            "content_type": "json",
            "insecure_ssl": "1"
        });
        // Register the shared secret so GitHub signs deliveries (HMAC-SHA256),
        // which the /apps/webhook endpoint then verifies.
        if let Ok(secret) = std::env::var("HERMES_GITHUB_WEBHOOK_SECRET") {
            if !secret.is_empty() {
                config["secret"] = serde_json::Value::String(secret);
            }
        }
        let webhook_payload = serde_json::json!({
            "name": "web",
            "active": true,
            "events": ["push", "pull_request"],
            "config": config
        });

        let url = format!("https://api.github.com/repos/{}/{}/hooks", owner, repo);
        let res = client.post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("User-Agent", "hermes-orchestrator")
            .header("Accept", "application/vnd.github+json")
            .json(&webhook_payload)
            .send()
            .await;

        match res {
            Ok(resp) => {
                if resp.status().is_success() {
                    tracing::info!(%owner, %repo, "Registered GitHub webhook at {}", webhook_url);
                } else if let Ok(err_txt) = resp.text().await {
                    tracing::warn!(%owner, %repo, "Failed to register GitHub webhook: {}", err_txt);
                }
            }
            Err(e) => {
                tracing::warn!(%owner, %repo, "Network error registering GitHub webhook: {}", e);
            }
        }
    });
}

pub async fn create_app(
    State(state): State<AppState>,
    headers: HeaderMap,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateAppRequest>,
) -> Result<(StatusCode, Json<AppDetailedResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    
    let slug = payload.name.trim().to_lowercase().replace(' ', "-");
    let branch = payload.branch_name.unwrap_or_else(|| "main".to_string());
    let internal_port = payload.internal_port.unwrap_or(3000);
    
    let external_port = match payload.external_port {
        Some(p) if p > 0 => p,
        _ => get_random_available_port(&state.pool).await?,
    };
    
    let container_name = crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}-{}", slug, branch, &instance_id.to_string()[..8]));
    let assigned_domain: Option<String> = None;

    // Resolve the in-cluster service alias (custom or auto) and ensure it is unique
    // within the workspace BEFORE creating anything (services share the namespace).
    let network_alias = match payload.network_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => crate::utils::string_gen::sanitize_k8s_name(custom),
        None => crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}", slug, branch)),
    };
    let alias_taken = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM app_instances ai JOIN apps a ON ai.app_id = a.id
         WHERE a.workspace_id = $1 AND ai.network_alias = $2)",
    )
    .bind(ws_id)
    .bind(&network_alias)
    .fetch_one(&state.pool)
    .await?;
    if alias_taken {
        return Err(AppError::Conflict(format!(
            "Numele de serviciu '{}' e deja folosit în acest workspace. Alege altul.",
            network_alias
        )));
    }

    let git_subpath = payload.git_subpath.as_deref().map(|s| s.trim().trim_matches('/')).filter(|s| !s.is_empty()).map(|s| s.to_string());

    // The chosen git credential (if any) must belong to this workspace.
    if let Some(cred_id) = payload.git_credential_id {
        let ok = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM git_credentials WHERE id = $1 AND workspace_id = $2)",
            cred_id, ws_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
        if !ok {
            return Err(AppError::Validation("Credențiala git nu aparține acestui workspace.".to_string()));
        }
    }

    sqlx::query!(
        "INSERT INTO apps (id, workspace_id, project_id, name, slug, git_repository, build_command, start_command, git_subpath, git_credential_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        app_id, ws_id, payload.project_id, payload.name, slug, payload.git_repository, payload.build_command, payload.start_command, git_subpath, payload.git_credential_id
    )
    .execute(&state.pool)
    .await?;

    sqlx::query!(
        "INSERT INTO app_instances (id, app_id, branch_name, instance_type, status, internal_port, assigned_domain, container_name, external_port)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        instance_id, app_id, branch, AppInstanceType::Production as AppInstanceType, AppStatus::Building as AppStatus, internal_port, assigned_domain, container_name, external_port
    )
    .execute(&state.pool)
    .await?;

    // Production instances do not auto-sleep by default (the column default is for
    // non-production); set it explicitly so a new production app stays up.
    sqlx::query("UPDATE app_instances SET auto_sleep_enabled = false WHERE id = $1")
        .bind(instance_id)
        .execute(&state.pool)
        .await?;

    // Persist the (already validated) network alias for the deploy path, and
    // optionally publish this app's URL into the project env pool so OTHER apps can
    // reference it (toggleable; key defaults to <SLUG>_URL).
    {
        sqlx::query("UPDATE app_instances SET network_alias = $1 WHERE id = $2")
            .bind(&network_alias)
            .bind(instance_id)
            .execute(&state.pool)
            .await?;

        if payload.publish_url != Some(false) {
            let app_url = format!("http://{}:{}", network_alias, internal_port);
            let url_key = payload
                .url_env_key
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_uppercase())
                .unwrap_or_else(|| format!("{}_URL", crate::utils::app_env::sanitize_key_fragment(&slug, "APP")));
            let _ = crate::utils::app_env::publish_project_env(
                &state.pool, ws_id, payload.project_id, &url_key, &app_url, false, "app", app_id,
            )
            .await;
        }
    }

    // Provision any environment variables supplied at creation time.
    if let Some(vars) = &payload.env_variables {
        for var in vars {
            let clean_key = var.key.trim().to_uppercase().replace(' ', "_");
            if clean_key.is_empty() {
                continue;
            }

            // Check if the key already exists in the project pool.
            let project_env = sqlx::query!(
                "SELECT id FROM project_env_variables WHERE project_id = $1 AND key = $2",
                payload.project_id, clean_key
            )
            .fetch_optional(&state.pool)
            .await?;

            if let Some(pe) = project_env {
                // Link to the project pool variable.
                sqlx::query!(
                    "INSERT INTO app_env_links (app_instance_id, project_env_id)
                     VALUES ($1, $2)
                     ON CONFLICT DO NOTHING",
                    instance_id, pe.id
                )
                .execute(&state.pool)
                .await?;
            } else {
                // Put it as a custom/local variable on the app.
                let is_secret = var.is_secret.unwrap_or(true);
                let (encrypted_value, nonce) = crate::utils::crypto::encrypt_env_value(&var.value)?;

                sqlx::query!(
                    "INSERT INTO environment_variables (id, workspace_id, app_instance_id, key, encrypted_value, nonce, is_secret)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (app_instance_id, key) DO NOTHING",
                    Uuid::new_v4(), ws_id, instance_id, clean_key, encrypted_value, nonce, is_secret
                )
                .execute(&state.pool)
                .await?;
            }
        }
    }

    // Opt the new instance into any project-pool env vars chosen at creation time.
    // Each id is validated to belong to this app's project before linking.
    if let Some(ids) = &payload.linked_project_env_ids {
        for project_env_id in ids {
            let belongs = sqlx::query_scalar!(
                "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE id = $1 AND project_id = $2)",
                project_env_id, payload.project_id
            )
            .fetch_one(&state.pool)
            .await?
            .unwrap_or(false);
            if !belongs {
                continue;
            }
            sqlx::query!(
                "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                instance_id, project_env_id
            )
            .execute(&state.pool)
            .await?;
        }
    }

    let _ = crate::utils::job_queue::enqueue_build(
        &state.pool,
        instance_id,
        payload.git_repository.clone(),
        branch.clone(),
        payload.build_command.clone(),
    ).await;

    // Auto-register the GitHub push webhook (auto-deploy on push). Shared helper so
    // the docker-compose split/import paths register it too.
    let webhook_host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:3000")
        .to_string();
    try_register_github_webhook(
        &state.pool,
        ws_id,
        claims.sub,
        payload.git_credential_id,
        &payload.git_repository,
        &webhook_host,
    )
    .await;

    let instances = vec![AppInstanceResponse {
        id: instance_id,
        branch_name: branch,
        instance_type: AppInstanceType::Production,
        status: AppStatus::Building,
        internal_port,
        assigned_domain,
        network_alias: Some(network_alias.clone()),
        container_name,
        external_port: Some(external_port),
        meta_data: serde_json::json!({}),
        cpu_limit: 0,
        memory_limit_mb: 0,
        replicas_min: 1,
        replicas_max: 1,
        autoscale_cpu_percent: 80,
        auto_sleep_enabled: false,
        auto_sleep_after_minutes: 30,
    }];

    Ok((
        StatusCode::CREATED,
        Json(AppDetailedResponse {
            id: app_id,
            project_id: payload.project_id,
            name: payload.name,
            slug,
            git_repository: payload.git_repository,
            namespace: format!("hermes-ws-{}", ws_id),
            instances,
            git_subpath,
            git_credential_id: payload.git_credential_id,
            build_command: payload.build_command,
            start_command: payload.start_command,
            created_at: chrono::Utc::now(),
        }),
    ))
}

pub async fn create_branch_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Json(payload): Json<CreateBranchRequest>,
) -> Result<(StatusCode, Json<AppInstanceResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Serialize quota-sensitive mutations per workspace (atomic check + insert).
    let _ws_guard = crate::utils::locks::acquire_workspace_lock(&state.pool, ws_id).await?;

    let app = sqlx::query_as::<_, App>("SELECT * FROM apps WHERE id = $1 AND workspace_id = $2")
        .bind(app_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Parent application not found.".to_string()))?;

    let instance_id = Uuid::new_v4();
    let container_name = crate::utils::string_gen::sanitize_k8s_name(&format!("hermes-app-{}-{}-{}", app.slug, payload.branch_name, &instance_id.to_string()[..8]));
    let assigned_domain: Option<String> = None;
    let internal_port = payload.internal_port.unwrap_or(3000);
    let external_port = match payload.external_port {
        Some(p) if p > 0 => p,
        _ => get_random_available_port(&state.pool).await?,
    };

    // Replica range: min >= 1, max >= min. min == max → fixed count; max > min → HPA.
    let replicas_min = payload.replicas_min.unwrap_or(1).max(1);
    let replicas_max = payload.replicas_max.unwrap_or(replicas_min).max(replicas_min);
    let autoscale_cpu_percent = payload.autoscale_cpu_percent.unwrap_or(80).clamp(1, 100);
    // Auto-sleep is opt-in: defaults off unless the caller explicitly enables it.
    let auto_sleep_enabled = payload.auto_sleep_enabled.unwrap_or(false);
    let auto_sleep_after_minutes = payload.auto_sleep_after_minutes.unwrap_or(30).clamp(1, 10080);

    // Quota checks account for full scale-out (per-replica limit × max replicas).
    let requested_mem = payload.memory_limit_mb.unwrap_or(0);
    if requested_mem > 0 {
        crate::utils::limits::check_workspace_memory_limit(
            &state.pool,
            ws_id,
            requested_mem * replicas_max as i64,
            None
        ).await?;
    }
    let requested_cpu = payload.cpu_limit.unwrap_or(0);
    if requested_cpu > 0 {
        crate::utils::limits::check_workspace_cpu_limit(&state.pool, ws_id, requested_cpu * replicas_max, None).await?;
    }

    sqlx::query!(
        "INSERT INTO app_instances (id, app_id, branch_name, instance_type, status, internal_port, assigned_domain, container_name, cpu_limit, memory_limit_mb, external_port, replicas_min, replicas_max)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        instance_id,
        app_id,
        payload.branch_name,
        payload.instance_type.clone() as AppInstanceType,
        AppStatus::Building as AppStatus,
        internal_port,
        assigned_domain,
        container_name,
        payload.cpu_limit.unwrap_or(0),
        payload.memory_limit_mb.unwrap_or(0),
        external_port,
        replicas_min,
        replicas_max
    )
    .execute(&state.pool)
    .await?;

    // The INSERT above leaves the scaling/auto-sleep columns at their defaults; set the
    // requested values separately (keeps the cached INSERT query untouched).
    sqlx::query(
        "UPDATE app_instances SET autoscale_cpu_percent = $1, auto_sleep_enabled = $2, auto_sleep_after_minutes = $3 WHERE id = $4",
    )
        .bind(autoscale_cpu_percent)
        .bind(auto_sleep_enabled)
        .bind(auto_sleep_after_minutes)
        .bind(instance_id)
        .execute(&state.pool)
        .await?;

    let _ = crate::utils::job_queue::enqueue_build(
        &state.pool,
        instance_id,
        app.git_repository.clone(),
        payload.branch_name.clone(),
        app.build_command.clone(),
    ).await;

    Ok((
        StatusCode::CREATED,
        Json(AppInstanceResponse {
            id: instance_id,
            branch_name: payload.branch_name,
            instance_type: payload.instance_type,
            status: AppStatus::Building,
            internal_port,
            assigned_domain,
            network_alias: None,
            container_name,
            external_port: Some(external_port),
            meta_data: serde_json::json!({}),
            cpu_limit: requested_cpu,
            memory_limit_mb: requested_mem,
            replicas_min,
            replicas_max,
            autoscale_cpu_percent,
            auto_sleep_enabled,
            auto_sleep_after_minutes,
        }),
    ))
}

pub async fn get_app_details(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<Json<AppDetailedResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app = sqlx::query_as::<_, App>("SELECT * FROM apps WHERE id = $1 AND workspace_id = $2")
        .bind(app_id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))?;

    let instances_records = sqlx::query_as::<_, AppInstance>("SELECT * FROM app_instances WHERE app_id = $1")
        .bind(app_id).fetch_all(&state.pool).await?;

    let instances = instances_records.into_iter().map(|inst| AppInstanceResponse {
        id: inst.id,
        branch_name: inst.branch_name,
        instance_type: inst.instance_type,
        status: inst.status,
        internal_port: inst.internal_port,
        assigned_domain: inst.assigned_domain,
        network_alias: inst.network_alias,
        container_name: inst.container_name,
        external_port: inst.external_port,
        meta_data: inst.meta_data,
        cpu_limit: inst.cpu_limit,
        memory_limit_mb: inst.memory_limit_mb,
        replicas_min: inst.replicas_min,
        replicas_max: inst.replicas_max,
        autoscale_cpu_percent: inst.autoscale_cpu_percent,
        auto_sleep_enabled: inst.auto_sleep_enabled,
        auto_sleep_after_minutes: inst.auto_sleep_after_minutes,
    }).collect();

    Ok(Json(AppDetailedResponse {
        id: app.id,
        project_id: app.project_id,
        name: app.name,
        slug: app.slug,
        git_repository: app.git_repository,
        namespace: format!("hermes-ws-{}", ws_id),
        instances,
        git_subpath: app.git_subpath,
        git_credential_id: app.git_credential_id,
        build_command: app.build_command,
        start_command: app.start_command,
        created_at: app.created_at,
    }))
}

pub async fn delete_app_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query_as::<_, AppInstance>(
        "SELECT ai.* FROM app_instances ai 
         JOIN apps a ON ai.app_id = a.id 
         WHERE ai.id = $1 AND a.workspace_id = $2"
    )
    .bind(instance_id).bind(ws_id).fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = instance.container_name.clone();
    let assigned_domain = instance.assigned_domain.clone();

    tokio::spawn(async move {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            if assigned_domain.is_some() {
                let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &container_name).await;
            }
            let _ = crate::utils::k8s::K8sManager::delete_app(&k8s_client, &namespace, &container_name).await;
            let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, &container_name).await;
        }
    });

    sqlx::query!("DELETE FROM environment_variables WHERE app_instance_id = $1", instance_id)
        .execute(&state.pool)
        .await?;

    // Tear down any custom domains attached to this instance (DNS, nginx, ingress + row).
    crate::controllers::domain_controller::purge_domains_for_target(&state.pool, ws_id, "app", instance_id).await;

    sqlx::query!("DELETE FROM app_instances WHERE id = $1", instance_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn stop_app_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query!(
        "SELECT ai.container_name, a.workspace_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2",
        instance_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = instance.container_name.clone();

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    crate::utils::k8s::K8sManager::scale_deployment(&k8s_client, &namespace, &container_name, 0).await?;

    sqlx::query!(
        "UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2",
        AppStatus::Stopped as AppStatus, instance_id
    )
    .execute(&state.pool)
    .await?;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
            workspace_id: ws_id,
            instance_id,
            container_name: container_name.clone(),
            status: "stopped".to_string(),
        }
    );

    Ok(StatusCode::OK)
}

pub async fn start_app_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query!(
        "SELECT ai.container_name, a.workspace_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2",
        instance_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = instance.container_name.clone();

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    // Restore the configured baseline, not a hard-coded 1 (a min-2 app must come back at 2).
    let replicas_min = sqlx::query_scalar::<_, i32>("SELECT replicas_min FROM app_instances WHERE id = $1")
        .bind(instance_id)
        .fetch_one(&state.pool)
        .await
        .unwrap_or(1)
        .max(1);
    crate::utils::k8s::K8sManager::scale_deployment(&k8s_client, &namespace, &container_name, replicas_min).await?;

    sqlx::query!(
        "UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2",
        AppStatus::Running as AppStatus, instance_id
    )
    .execute(&state.pool)
    .await?;

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
            workspace_id: ws_id,
            instance_id,
            container_name: container_name.clone(),
            status: "running".to_string(),
        }
    );

    Ok(StatusCode::OK)
}

/// Redeploy = full rebuild from Git (fresh clone + image build + deploy), using the
/// instance's current repo/branch/build command. For re-applying the already-built
/// image without a rebuild, use `reload_app_instance` instead.
pub async fn redeploy_app_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let meta = sqlx::query!(
        "SELECT ai.id, ai.branch_name, a.git_repository, a.build_command
         FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2",
        instance_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let busy = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_builds WHERE app_instance_id = $1 AND status = 'building')",
        meta.id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if busy {
        return Err(AppError::Conflict("Există deja un build în curs pentru această instanță.".to_string()));
    }

    sqlx::query!(
        "UPDATE app_instances SET status = 'building', updated_at = now() WHERE id = $1",
        meta.id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::utils::job_queue::enqueue_build(
        &state.pool,
        meta.id,
        meta.git_repository.clone(),
        meta.branch_name.clone(),
        meta.build_command.clone(),
    ).await;

    Ok(StatusCode::ACCEPTED)
}

/// Reload = re-apply the already-built image with freshly-resolved config/env, no
/// rebuild (the previous behavior of "redeploy"). Picks up env/limit/domain changes.
pub async fn reload_app_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query!(
        "SELECT ai.id, a.workspace_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2",
        instance_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let _ = crate::utils::job_queue::enqueue_deploy(&state.pool, instance.id, None).await;

    Ok(StatusCode::OK)
}

pub async fn configure_serverless(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<ConfigureServerlessRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query!(
        "SELECT ai.meta_data, a.workspace_id FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2",
        instance_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application branch instance not found.".to_string()))?;

    let mut current_meta = instance.meta_data;
    if !current_meta.is_object() {
        current_meta = serde_json::json!({});
    }

    if let Some(obj) = current_meta.as_object_mut() {
        obj.insert("knative_enabled".to_string(), serde_json::Value::Bool(payload.enabled));
        obj.insert("minScale".to_string(), serde_json::Value::Number(payload.min_scale.into()));
        obj.insert("maxScale".to_string(), serde_json::Value::Number(payload.max_scale.into()));
        obj.insert("targetConcurrency".to_string(), serde_json::Value::Number(payload.target_concurrency.into()));
    }

    sqlx::query!(
        "UPDATE app_instances SET meta_data = $1, updated_at = now() WHERE id = $2",
        current_meta, instance_id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::utils::job_queue::enqueue_deploy(&state.pool, instance_id, None).await;

    Ok(StatusCode::OK)
}

#[derive(Debug, Deserialize)]
pub struct LogQuery {
    pub previous: Option<bool>,
}

pub async fn stream_instance_stats(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query_as::<_, AppInstance>(
        "SELECT ai.* FROM app_instances ai 
         JOIN apps a ON ai.app_id = a.id 
         WHERE ai.id = $1 AND a.workspace_id = $2"
    )
    .bind(instance_id).bind(ws_id).fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Application instance not found.".to_string()))?;

    let k8s_client = crate::utils::k8s::K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = instance.container_name.clone();

    let sse_stream = async_stream::stream! {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        // Cumulative counters the UI diffs into a CPU% (container-ns / wall-ns).
        let mut cpu_container_ns = 0u64;
        let mut cpu_system_ns = 0u64;
        let mut last_tick = std::time::Instant::now();

        loop {
            interval.tick().await;

            let mut got_real_metrics = false;
            let mut ram_usage = 0u64;
            let mut cpu_usage = 0u64;

            let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client.clone(), &namespace);
            let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));
            if let Ok(pod_list) = pods_api.list(&lp).await {
                if let Some(pod) = pod_list.items.first() {
                    if let Some(ref pod_name) = pod.metadata.name {
                        let request_url = format!("/apis/metrics.k8s.io/v1beta1/namespaces/{}/pods/{}", namespace, pod_name);
                        if let Ok(request) = axum::http::Request::get(&request_url).body(vec![]) {
                            if let Ok(response_val) = k8s_client.request::<serde_json::Value>(request).await {
                                if let Some(containers) = response_val.get("containers").and_then(|c| c.as_array()) {
                                    if let Some(c) = containers.first() {
                                        if let Some(usage) = c.get("usage") {
                                            if let Some(mem_str) = usage.get("memory").and_then(|m| m.as_str()) {
                                                ram_usage = crate::utils::quantity::parse_memory_bytes(mem_str);
                                            }
                                            if let Some(cpu_str) = usage.get("cpu").and_then(|c| c.as_str()) {
                                                cpu_usage = crate::utils::quantity::parse_cpu_nanocores(cpu_str);
                                            }
                                            got_real_metrics = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let elapsed_ns = last_tick.elapsed().as_nanos() as u64;
            last_tick = std::time::Instant::now();

            // NEVER fabricate. Emit real values with `available: true`, or signal
            // `available: false` so the UI shows "unavailable" instead of fiction.
            let stats_payload = if got_real_metrics {
                cpu_system_ns += elapsed_ns;
                // cpu_usage is nanocores (cores * 1e9); CPU-ns consumed this interval
                // = nanocores * elapsed_ns / 1e9.
                cpu_container_ns += ((cpu_usage as u128 * elapsed_ns as u128) / 1_000_000_000u128) as u64;
                serde_json::json!({
                    "available": true,
                    "memoryBytes": ram_usage,
                    "cpuSystem": cpu_system_ns,
                    "cpuContainer": cpu_container_ns
                })
            } else {
                serde_json::json!({ "available": false })
            };

            yield Ok::<_, Infallible>(Event::default().data(stats_payload.to_string()));
        }
    };

    Ok(Sse::new(sse_stream))
}

/// Constant-time HMAC-SHA256 verification of a GitHub `X-Hub-Signature-256` header.
fn verify_github_signature(secret: &[u8], body: &[u8], signature_header: &str) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let Some(hex) = signature_header.strip_prefix("sha256=") else { return false; };
    if hex.len() % 2 != 0 { return false; }
    let expected: Option<Vec<u8>> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect();
    let Some(expected) = expected else { return false; };

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else { return false; };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

pub async fn handle_github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, AppError> {
    // When a webhook secret is configured, every delivery must carry a valid
    // HMAC-SHA256 signature — otherwise forged payloads could trigger builds or
    // create/delete instances. With no secret set, validation is skipped (a
    // warning is logged) so existing deployments keep working.
    match std::env::var("HERMES_GITHUB_WEBHOOK_SECRET") {
        Ok(secret) if !secret.is_empty() => {
            let signature = headers
                .get("x-hub-signature-256")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| AppError::Auth("Missing webhook signature.".to_string()))?;
            if !verify_github_signature(secret.as_bytes(), &body, signature) {
                return Err(AppError::Auth("Invalid webhook signature.".to_string()));
            }
        }
        _ => {
            tracing::warn!("HERMES_GITHUB_WEBHOOK_SECRET not set — webhook signature validation is DISABLED.");
        }
    }

    let payload: GitHubWebhookPayload = serde_json::from_slice(&body)
        .map_err(|e| AppError::Validation(format!("Invalid webhook payload: {}", e)))?;

    let mut target_branch = None;

    if let Some(ref action) = payload.action {
        if action == "closed" {
            if let Some(ref pr) = payload.pull_request {
                target_branch = Some(pr.head.r#ref.clone());
            }
        }
    }

    if let Some(ref ref_type) = payload.ref_type {
        if ref_type == "branch" {
            if let Some(ref br) = payload.r#ref {
                target_branch = Some(br.replace("refs/heads/", ""));
            }
        }
    }

    if let Some(branch_name) = target_branch {
        let instances = sqlx::query!(
            "SELECT id, assigned_domain, container_name FROM app_instances 
             WHERE branch_name = $1 AND instance_type != 'production'",
            &branch_name
        )
        .fetch_all(&state.pool)
        .await?;

        for inst in instances {
            let pool_clone = state.pool.clone();
            let inst_id = inst.id;
            let container_name = inst.container_name.clone();
            let assigned_domain = inst.assigned_domain.clone();

            tokio::spawn(async move {
                let meta = sqlx::query!(
                    "SELECT a.workspace_id FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1",
                    inst_id
                )
                .fetch_optional(&pool_clone)
                .await;

                if let Ok(Some(m)) = meta {
                    let namespace = format!("hermes-ws-{}", m.workspace_id);
                    if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                        if assigned_domain.is_some() {
                            let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &container_name).await;
                        }
                        let _ = crate::utils::k8s::K8sManager::delete_app(&k8s_client, &namespace, &container_name).await;
                        let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, &container_name).await;
                    }
                }
                
                let _ = sqlx::query!("DELETE FROM environment_variables WHERE app_instance_id = $1", inst_id).execute(&pool_clone).await;
                let _ = sqlx::query!("DELETE FROM app_instances WHERE id = $1", inst_id).execute(&pool_clone).await;
            });
        }
    }

    // Preview environments: PR opened/reopened → spin up a preview instance for
    // the PR's head branch on every app that tracks this repository. The matching
    // teardown already happens above when the PR is closed.
    if let (Some(action), Some(pr), Some(repo)) = (&payload.action, &payload.pull_request, &payload.repository) {
        if action == "opened" || action == "reopened" {
            let branch_name = pr.head.r#ref.clone();
            let ssh_url = repo.ssh_url.clone().unwrap_or_default().trim().to_lowercase();
            let clone_url = repo.clone_url.clone().unwrap_or_default().trim().to_lowercase();

            let apps = sqlx::query!(
                "SELECT id, slug, git_repository, build_command, workspace_id FROM apps"
            )
            .fetch_all(&state.pool)
            .await?;

            for app in apps {
                let db_repo = app.git_repository.trim().to_lowercase();
                let is_match = db_repo == ssh_url
                    || db_repo == clone_url
                    || db_repo.replace(".git", "") == ssh_url.replace(".git", "")
                    || db_repo.replace(".git", "") == clone_url.replace(".git", "");
                if !is_match {
                    continue;
                }

                // One instance per app+branch: skip if it already exists.
                let exists = sqlx::query_scalar!(
                    "SELECT EXISTS(SELECT 1 FROM app_instances WHERE app_id = $1 AND branch_name = $2)",
                    app.id, branch_name
                )
                .fetch_one(&state.pool)
                .await?
                .unwrap_or(false);
                if exists {
                    continue;
                }

                // Inherit the internal port from the production instance.
                let internal_port = sqlx::query_scalar!(
                    "SELECT internal_port FROM app_instances WHERE app_id = $1 AND instance_type = 'production' LIMIT 1",
                    app.id
                )
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or(3000);

                let external_port = get_random_available_port(&state.pool).await?;
                let instance_id = Uuid::new_v4();
                let container_name = crate::utils::string_gen::sanitize_k8s_name(
                    &format!("hermes-app-{}-{}-{}", app.slug, branch_name, &instance_id.to_string()[..8])
                );

                sqlx::query!(
                    "INSERT INTO app_instances (id, app_id, branch_name, instance_type, status, internal_port, assigned_domain, container_name, cpu_limit, memory_limit_mb, external_port)
                     VALUES ($1, $2, $3, $4, $5, $6, NULL, $7, 0, 0, $8)",
                    instance_id,
                    app.id,
                    branch_name,
                    AppInstanceType::Preview as AppInstanceType,
                    AppStatus::Building as AppStatus,
                    internal_port,
                    container_name,
                    external_port
                )
                .execute(&state.pool)
                .await?;

                crate::utils::event_broadcaster::broadcast_event(
                    crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                        workspace_id: app.workspace_id,
                        instance_id,
                        container_name,
                        status: "building".to_string(),
                    }
                );

                let _ = crate::utils::job_queue::enqueue_build(
                    &state.pool,
                    instance_id,
                    app.git_repository.clone(),
                    branch_name.clone(),
                    app.build_command.clone(),
                ).await;
            }
        }
    }

    // Handle GitHub Push Event for Auto-Rebuild/Update
    if payload.action.is_none() {
        if let Some(ref_str) = payload.r#ref {
            if ref_str.starts_with("refs/heads/") {
                let branch_name = ref_str.replace("refs/heads/", "");
                if let Some(repo) = payload.repository {
                    let ssh_url = repo.ssh_url.unwrap_or_default();
                    let clone_url = repo.clone_url.unwrap_or_default();

                    let matches = sqlx::query!(
                        "SELECT ai.id, a.git_repository, a.build_command, a.workspace_id, ai.container_name
                         FROM app_instances ai
                         JOIN apps a ON ai.app_id = a.id
                         WHERE ai.branch_name = $1",
                        branch_name
                    )
                    .fetch_all(&state.pool)
                    .await?;

                    for record in matches {
                        let db_repo = record.git_repository.trim().to_lowercase();
                        let clean_ssh = ssh_url.trim().to_lowercase();
                        let clean_clone = clone_url.trim().to_lowercase();

                        let is_match = db_repo == clean_ssh
                            || db_repo == clean_clone
                            || db_repo.replace(".git", "") == clean_ssh.replace(".git", "")
                            || db_repo.replace(".git", "") == clean_clone.replace(".git", "");

                        if is_match {
                            let inst_id = record.id;
                            let git_repo = record.git_repository;
                            let branch_clone = branch_name.clone();
                            let build_cmd = record.build_command;

                            // Mark instance status as Building
                            let _ = sqlx::query!(
                                "UPDATE app_instances SET status = $1, updated_at = now() WHERE id = $2",
                                AppStatus::Building as AppStatus, inst_id
                            )
                            .execute(&state.pool)
                            .await;

                            crate::utils::event_broadcaster::broadcast_event(
                                crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                                    workspace_id: record.workspace_id,
                                    instance_id: inst_id,
                                    container_name: record.container_name.clone(),
                                    status: "building".to_string(),
                                }
                            );

                            // Trigger rebuild (durable queue)
                            let _ = crate::utils::job_queue::enqueue_build(
                                &state.pool, inst_id, git_repo, branch_clone, build_cmd,
                            ).await;
                        }
                    }
                }
            }
        }
    }

    Ok(StatusCode::OK)
}

/// Global build/work queue: app builds (queued/building) + databases provisioning
/// + serverless builds, oldest first. Super admins see all workspaces; everyone
/// else sees their active workspace.
pub async fn list_build_queue(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<BuildQueueItem>>, AppError> {
    let filter_ws: Option<Uuid> = if claims.is_super_admin {
        None
    } else {
        Some(claims.current_workspace_id.ok_or_else(|| {
            AppError::Validation("No active workspace selected.".to_string())
        })?)
    };

    let mut items: Vec<BuildQueueItem> = Vec::new();

    // App instances that aren't available yet. We key off the *instance* status
    // ('building') rather than the build-row status, because an instance stays
    // 'building' through the whole lifecycle — image build AND the post-build
    // deploy + readiness gate (monitor_deploy_health flips it to 'running' only
    // once the pod is actually Ready). Keying off app_builds.status dropped the
    // item the instant the image finished, well before the app was available.
    let app_rows = sqlx::query!(
        r#"SELECT ai.id AS instance_id, a.id AS app_id, a.name AS app_name, a.project_id, p.name AS project_name,
                  a.workspace_id, w.name AS workspace_name, ai.branch_name,
                  lb.status AS latest_build_status,
                  COALESCE(lb.created_at, ai.updated_at) AS "started_at!"
           FROM app_instances ai
           JOIN apps a ON ai.app_id = a.id
           JOIN projects p ON a.project_id = p.id
           JOIN workspaces w ON a.workspace_id = w.id
           LEFT JOIN LATERAL (
               SELECT status, created_at FROM app_builds ab
               WHERE ab.app_instance_id = ai.id ORDER BY ab.created_at DESC LIMIT 1
           ) lb ON true
           WHERE ai.status = 'building'
             AND ($1::uuid IS NULL OR a.workspace_id = $1)"#,
        filter_ws
    )
    .fetch_all(&state.pool)
    .await?;
    for r in app_rows {
        // queued → waiting for a build slot; building → image building;
        // anything else (success / none) → image done, now deploying + warming up.
        let status = match r.latest_build_status.as_deref() {
            Some("queued") => "queued",
            Some("building") => "building",
            _ => "deploying",
        };
        items.push(BuildQueueItem {
            id: r.instance_id,
            kind: "app".to_string(),
            resource_id: r.app_id,
            name: r.app_name,
            detail: Some(r.branch_name),
            project_id: r.project_id,
            project_name: r.project_name,
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            status: status.to_string(),
            // Elapsed since the current build/deploy started (not instance creation,
            // which would show days for a rebuild of an old instance).
            created_at: r.started_at,
        });
    }

    // Databases currently provisioning.
    let db_rows = sqlx::query!(
        r#"SELECT d.id, d.name, d.type::text AS "db_type!", d.project_id, p.name AS project_name,
                  d.workspace_id, w.name AS workspace_name, d.created_at
           FROM databases d
           JOIN projects p ON d.project_id = p.id
           JOIN workspaces w ON d.workspace_id = w.id
           WHERE d.status = 'provisioning'
             AND ($1::uuid IS NULL OR d.workspace_id = $1)"#,
        filter_ws
    )
    .fetch_all(&state.pool)
    .await?;
    for r in db_rows {
        items.push(BuildQueueItem {
            id: r.id,
            kind: "database".to_string(),
            resource_id: r.id,
            name: r.name,
            detail: Some(r.db_type),
            project_id: r.project_id,
            project_name: r.project_name,
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            status: "provisioning".to_string(),
            created_at: r.created_at,
        });
    }

    // Serverless instances currently building.
    let fn_rows = sqlx::query!(
        r#"SELECT f.id, f.name, f.runtime, f.project_id, p.name AS project_name,
                  f.workspace_id, w.name AS workspace_name, f.updated_at
           FROM serverless_instances f
           JOIN projects p ON f.project_id = p.id
           JOIN workspaces w ON f.workspace_id = w.id
           WHERE f.status = 'building'
             AND ($1::uuid IS NULL OR f.workspace_id = $1)"#,
        filter_ws
    )
    .fetch_all(&state.pool)
    .await?;
    for r in fn_rows {
        items.push(BuildQueueItem {
            id: r.id,
            kind: "serverless".to_string(),
            resource_id: r.id,
            name: r.name,
            detail: Some(r.runtime),
            project_id: r.project_id,
            project_name: r.project_name,
            workspace_id: r.workspace_id,
            workspace_name: r.workspace_name,
            status: "building".to_string(),
            created_at: r.updated_at,
        });
    }

    items.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(Json(items))
}

pub async fn list_app_builds(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<BuildResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;

    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!("SELECT COUNT(*) FROM app_builds WHERE app_id = $1", app_id)
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(0);

    let records = sqlx::query!(
        "SELECT ab.id, ab.app_id, ab.app_instance_id, ab.status, ab.phase, ab.failure_reason, ab.failure_category, ab.created_at, ai.branch_name, ab.commit_message, ab.commit_sha, ab.duration_sec, ab.image_tag, ai.current_image_tag
         FROM app_builds ab
         JOIN app_instances ai ON ab.app_instance_id = ai.id
         WHERE ab.app_id = $1
         ORDER BY ab.created_at DESC
         LIMIT $2 OFFSET $3",
        app_id, page_size, offset
    )
    .fetch_all(&state.pool)
    .await?;

    let items: Vec<BuildResponse> = records
        .into_iter()
        .map(|r| {
            // The build whose image matches the instance's deployed image is "live".
            let is_live = match (&r.image_tag, &r.current_image_tag) {
                (Some(img), Some(cur)) => img == cur,
                _ => false,
            };
            BuildResponse {
                id: r.id,
                app_id: r.app_id,
                app_instance_id: r.app_instance_id,
                branch_name: r.branch_name,
                status: r.status,
                phase: r.phase,
                failure_reason: r.failure_reason,
                failure_category: r.failure_category,
                created_at: r.created_at,
                commit_message: r.commit_message,
                commit_sha: r.commit_sha,
                duration_sec: r.duration_sec,
                image_tag: r.image_tag,
                is_live,
            }
        })
        .collect();

    Ok(Json(Paginated::new(items, total, page, page_size)))
}
 
pub async fn get_build_details(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, build_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<BuildDetailResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;
 
    let app_exists = sqlx::query!("SELECT id FROM apps WHERE id = $1 AND workspace_id = $2", app_id, ws_id)
        .fetch_optional(&state.pool)
        .await?;
 
    if app_exists.is_none() {
        return Err(AppError::NotFound("Application not found in this workspace.".to_string()));
    }
 
    let record = sqlx::query!(
        "SELECT ab.id, ab.app_id, ab.app_instance_id, ab.status, ab.phase, ab.failure_reason, ab.failure_category, ab.logs, ab.created_at, ai.branch_name, ab.commit_message, ab.commit_sha, ab.duration_sec
         FROM app_builds ab
         JOIN app_instances ai ON ab.app_instance_id = ai.id
         WHERE ab.id = $1 AND ab.app_id = $2",
        build_id, app_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build log record not found.".to_string()))?;

    let mut logs = record.logs;
    if record.status == "building" {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let namespace = format!("hermes-ws-{}", ws_id);
            let builder_pod_name = format!("hermes-builder-{}", record.app_instance_id);
            let pods: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client, &namespace);
            
            if let Ok(_pod) = pods.get(&builder_pod_name).await {
                let mut live_logs = String::new();
                live_logs.push_str("=========================================\n");
                live_logs.push_str(" ETAPA 1: DESCĂRCARE COD (GIT CLONE) (LIVE)\n");
                live_logs.push_str("=========================================\n");
                
                let cloner_params = kube::api::LogParams {
                    container: Some("cloner".to_string()),
                    ..Default::default()
                };
                match pods.logs(&builder_pod_name, &cloner_params).await {
                    Ok(l) => live_logs.push_str(&l),
                    Err(_) => live_logs.push_str("Se descarcă codul sau se pregătește containerul...\n"),
                }

                live_logs.push_str("\n\n=========================================\n");
                live_logs.push_str(" ETAPA 2: CONSTRUIRE IMAGINE (KANIKO) (LIVE)\n");
                live_logs.push_str("=========================================\n");
                
                let kaniko_params = kube::api::LogParams {
                    container: Some("kaniko".to_string()),
                    ..Default::default()
                };
                match pods.logs(&builder_pod_name, &kaniko_params).await {
                    Ok(l) => live_logs.push_str(&l),
                    Err(_) => live_logs.push_str("Se construiește imaginea docker (Kaniko) sau se așteaptă containerul...\n"),
                }
                logs = live_logs;
            }
        }
    }
 
    Ok(Json(BuildDetailResponse {
        id: record.id,
        app_id: record.app_id,
        app_instance_id: record.app_instance_id,
        branch_name: record.branch_name,
        status: record.status,
        phase: record.phase,
        failure_reason: record.failure_reason,
        failure_category: record.failure_category,
        logs,
        created_at: record.created_at,
        commit_message: record.commit_message,
        commit_sha: record.commit_sha,
        duration_sec: record.duration_sec,
    }))
}

/// Cancel an in-progress build: mark it cancelled and tear down its builder pod.
pub async fn cancel_build(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, build_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let build = sqlx::query!(
        "SELECT ab.status, ab.app_instance_id, a.workspace_id
         FROM app_builds ab
         JOIN apps a ON ab.app_id = a.id
         WHERE ab.id = $1 AND ab.app_id = $2 AND a.workspace_id = $3",
        build_id, app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build not found for this application.".to_string()))?;

    if build.status != "building" {
        return Err(AppError::Conflict("Doar build-urile în curs pot fi anulate.".to_string()));
    }

    // Signal the running build loop to stop, and reflect it on the build record.
    sqlx::query!(
        "UPDATE app_builds SET status = 'cancelled', phase = 'cancelled', failure_reason = $2, failure_category = 'CANCELLED' WHERE id = $1",
        build_id, "Build anulat manual de utilizator."
    )
    .execute(&state.pool)
    .await?;

    sqlx::query!(
        "UPDATE app_instances SET status = 'stopped', updated_at = now() WHERE id = $1",
        build.app_instance_id
    )
    .execute(&state.pool)
    .await?;

    // Tear down the builder pod immediately.
    let namespace = format!("hermes-ws-{}", build.workspace_id);
    let pod_name = format!("hermes-builder-{}", build.app_instance_id);
    tokio::spawn(async move {
        if let Ok(client) = crate::utils::k8s::K8sManager::get_client().await {
            let pods: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(client, &namespace);
            let _ = pods.delete(&pod_name, &kube::api::DeleteParams::default()).await;
        }
    });

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::BuildStatusChanged {
            workspace_id: build.workspace_id,
            build_id,
            app_id,
            status: "cancelled".to_string(),
            phase: Some("cancelled".to_string()),
        }
    );

    Ok(StatusCode::OK)
}

/// Roll back an instance to the image produced by a previous successful build.
pub async fn rollback_build(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, build_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let build = sqlx::query!(
        "SELECT ab.status, ab.image_tag, ab.app_instance_id
         FROM app_builds ab
         JOIN apps a ON ab.app_id = a.id
         WHERE ab.id = $1 AND ab.app_id = $2 AND a.workspace_id = $3",
        build_id, app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build not found for this application.".to_string()))?;

    if build.status != "succeeded" {
        return Err(AppError::Conflict("Se poate face rollback doar la un build reușit.".to_string()));
    }
    let image_tag = build.image_tag.ok_or_else(|| {
        AppError::Conflict("Acest build nu are o imagine asociată (build vechi, dinainte de tagurile imutabile).".to_string())
    })?;

    let busy = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_builds WHERE app_instance_id = $1 AND status = 'building')",
        build.app_instance_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if busy {
        return Err(AppError::Conflict("Există deja un build/deploy în curs pentru această instanță.".to_string()));
    }

    // Point the instance at the chosen image and redeploy it.
    sqlx::query!(
        "UPDATE app_instances SET current_image_tag = $1, status = 'building', updated_at = now() WHERE id = $2",
        image_tag, build.app_instance_id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::utils::job_queue::enqueue_deploy(&state.pool, build.app_instance_id, Some(image_tag)).await;

    Ok(StatusCode::ACCEPTED)
}

/// Re-run a build with the same configuration (same repo, branch and build command).
pub async fn retry_build(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, build_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let meta = sqlx::query!(
        "SELECT ab.app_instance_id, ai.branch_name, a.git_repository, a.build_command
         FROM app_builds ab
         JOIN app_instances ai ON ab.app_instance_id = ai.id
         JOIN apps a ON ab.app_id = a.id
         WHERE ab.id = $1 AND ab.app_id = $2 AND a.workspace_id = $3",
        build_id, app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build not found for this application.".to_string()))?;

    let busy = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_builds WHERE app_instance_id = $1 AND status = 'building')",
        meta.app_instance_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if busy {
        return Err(AppError::Conflict("Există deja un build în curs pentru această instanță.".to_string()));
    }

    sqlx::query!(
        "UPDATE app_instances SET status = 'building', updated_at = now() WHERE id = $1",
        meta.app_instance_id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::utils::job_queue::enqueue_build(
        &state.pool,
        meta.app_instance_id,
        meta.git_repository,
        meta.branch_name,
        meta.build_command,
    ).await;

    Ok(StatusCode::ACCEPTED)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsQuery {
    pub metric: String,
    pub range: Option<String>,
}

pub async fn get_instance_metrics(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(query): Query<MetricsQuery>,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<MetricsHistoryResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query_as::<_, AppInstance>(
        "SELECT ai.* FROM app_instances ai 
         JOIN apps a ON ai.app_id = a.id 
         WHERE ai.id = $1 AND a.workspace_id = $2"
    )
    .bind(instance_id).bind(ws_id).fetch_optional(&state.pool).await?
    .ok_or_else(|| AppError::NotFound("Application instance not found.".to_string()))?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let container_name = instance.container_name.clone();
    let range = query.range.unwrap_or_else(|| "1h".to_string());

    let (timestamps, values, simulated) = crate::utils::prometheus::get_historical_metrics(
        &namespace,
        &container_name,
        &query.metric,
        &range,
        "app", // apps don't use the engine-specific db_* queries
    ).await?;

    Ok(Json(MetricsHistoryResponse {
        timestamps,
        values,
        simulated,
    }))
}

pub async fn list_project_apps(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<AppDetailedResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify project exists and belongs to workspace
    let project_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
        project_id,
        ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !project_exists {
        return Err(AppError::NotFound("Project not found in this workspace.".to_string()));
    }

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM apps WHERE project_id = $1 AND workspace_id = $2",
        project_id, ws_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let apps_records = sqlx::query_as::<_, App>(
        "SELECT * FROM apps WHERE project_id = $1 AND workspace_id = $2 ORDER BY created_at DESC LIMIT $3 OFFSET $4"
    )
    .bind(project_id)
    .bind(ws_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let mut items = Vec::new();
    for app in apps_records {
        let instances_records = sqlx::query_as::<_, AppInstance>(
            "SELECT * FROM app_instances WHERE app_id = $1 ORDER BY created_at DESC"
        )
        .bind(app.id)
        .fetch_all(&state.pool)
        .await?;

        let instances = instances_records.into_iter().map(|inst| AppInstanceResponse {
            id: inst.id,
            branch_name: inst.branch_name,
            instance_type: inst.instance_type,
            status: inst.status,
            internal_port: inst.internal_port,
            assigned_domain: inst.assigned_domain,
            network_alias: inst.network_alias,
            container_name: inst.container_name,
            external_port: inst.external_port,
            meta_data: inst.meta_data,
            cpu_limit: inst.cpu_limit,
            memory_limit_mb: inst.memory_limit_mb,
            replicas_min: inst.replicas_min,
            replicas_max: inst.replicas_max,
            autoscale_cpu_percent: inst.autoscale_cpu_percent,
            auto_sleep_enabled: inst.auto_sleep_enabled,
            auto_sleep_after_minutes: inst.auto_sleep_after_minutes,
        }).collect();

        items.push(AppDetailedResponse {
            id: app.id,
            project_id: app.project_id,
            name: app.name,
            slug: app.slug,
            git_repository: app.git_repository,
            namespace: format!("hermes-ws-{}", ws_id),
            instances,
            git_subpath: app.git_subpath,
            git_credential_id: app.git_credential_id,
            build_command: app.build_command,
            start_command: app.start_command,
            created_at: app.created_at,
        });
    }

    Ok(Json(Paginated::new(items, total, page, page_size)))
}

pub async fn update_instance_settings(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateInstanceSettingsRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Serialize quota-sensitive mutations per workspace (atomic check + update).
    let _ws_guard = crate::utils::locks::acquire_workspace_lock(&state.pool, ws_id).await?;

    // Verify application belongs to this workspace
    // Authorization check: the app must belong to the caller's workspace.
    let _app_meta = sqlx::query!(
        "SELECT id FROM apps WHERE id = $1 AND workspace_id = $2",
        app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found in this workspace.".to_string()))?;

    // Verify instance belongs to this app
    let instance_exists = sqlx::query!(
        "SELECT id, container_name FROM app_instances WHERE id = $1 AND app_id = $2",
        instance_id, app_id
    )
    .fetch_optional(&state.pool)
    .await?;

    if instance_exists.is_none() {
        return Err(AppError::NotFound("Application instance not found.".to_string()));
    }

    if let Some(requested_mem) = payload.memory_limit_mb {
        if requested_mem > 0 {
            crate::utils::limits::check_workspace_memory_limit(
                &state.pool,
                ws_id,
                requested_mem,
                Some(instance_id)
            ).await?;
        }
    }
    if let Some(requested_cpu) = payload.cpu_limit {
        if requested_cpu > 0 {
            crate::utils::limits::check_workspace_cpu_limit(
                &state.pool,
                ws_id,
                requested_cpu,
                Some(instance_id)
            ).await?;
        }
    }

    let external_port = match payload.external_port {
        Some(p) if p > 0 => p,
        _ => get_random_available_port(&state.pool).await?,
    };

    let rebuild_needed = payload.build_command.is_some() || payload.start_command.is_some() || payload.internal_port.is_some();

    // Update apps table if build/start commands are passed
    if payload.build_command.is_some() || payload.start_command.is_some() {
        sqlx::query!(
            "UPDATE apps 
             SET build_command = COALESCE($1, build_command),
                 start_command = COALESCE($2, start_command),
                 updated_at = now() 
             WHERE id = $3 AND workspace_id = $4",
            payload.build_command,
            payload.start_command,
            app_id,
            ws_id
        )
        .execute(&state.pool)
        .await?;
    }

    // Replica range validation (only when both bounds are supplied).
    if let (Some(mn), Some(mx)) = (payload.replicas_min, payload.replicas_max) {
        if mn < 1 || mx < mn {
            return Err(AppError::Validation(
                "Replica range invalid: min must be >= 1 and max >= min.".to_string(),
            ));
        }
    }

    // Update settings in database
    sqlx::query!(
        "UPDATE app_instances
         SET cpu_limit = COALESCE($1, cpu_limit),
             memory_limit_mb = COALESCE($2, memory_limit_mb),
             internal_port = COALESCE($3, internal_port),
             port_is_auto = CASE WHEN $3 IS NOT NULL THEN false ELSE port_is_auto END,
             external_port = $4,
             replicas_min = COALESCE($6, replicas_min),
             replicas_max = COALESCE($7, replicas_max),
             status = 'building',
             updated_at = now()
         WHERE id = $5",
        payload.cpu_limit,
        payload.memory_limit_mb,
        payload.internal_port,
        external_port,
        instance_id,
        payload.replicas_min,
        payload.replicas_max
    )
    .execute(&state.pool)
    .await?;

    // Scaling/auto-sleep columns are updated separately to keep the cached query intact.
    if let Some(pct) = payload.autoscale_cpu_percent {
        sqlx::query("UPDATE app_instances SET autoscale_cpu_percent = $1 WHERE id = $2")
            .bind(pct.clamp(1, 100))
            .bind(instance_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(enabled) = payload.auto_sleep_enabled {
        sqlx::query("UPDATE app_instances SET auto_sleep_enabled = $1 WHERE id = $2")
            .bind(enabled)
            .bind(instance_id)
            .execute(&state.pool)
            .await?;
    }
    if let Some(mins) = payload.auto_sleep_after_minutes {
        sqlx::query("UPDATE app_instances SET auto_sleep_after_minutes = $1 WHERE id = $2")
            .bind(mins.clamp(1, 10080))
            .bind(instance_id)
            .execute(&state.pool)
            .await?;
    }

    // Trigger rebuild or redeploy via the durable queue (survives restarts).
    if rebuild_needed {
        if let Ok(meta) = sqlx::query!(
            "SELECT git_repository, branch_name FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1",
            instance_id
        )
        .fetch_one(&state.pool)
        .await
        {
            let build_command = sqlx::query_scalar!("SELECT build_command FROM apps WHERE id = $1", app_id)
                .fetch_one(&state.pool)
                .await
                .unwrap_or(None);
            let _ = crate::utils::job_queue::enqueue_build(
                &state.pool, instance_id, meta.git_repository, meta.branch_name, build_command,
            ).await;
        }
    } else {
        // Redeploy the existing image with the updated limits & ports.
        let _ = crate::utils::job_queue::enqueue_deploy(&state.pool, instance_id, None).await;
    }

    Ok(StatusCode::OK)
}

pub async fn delete_app(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(app_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Check if the app exists and belongs to the workspace
    let app = sqlx::query!(
        "SELECT id, name FROM apps WHERE id = $1 AND workspace_id = $2",
        app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))?;

    // Remove this app's published URL from the project env pool (+ unlink consumers).
    let _ = crate::utils::app_env::unpublish_project_env(&state.pool, "app", app.id).await;

    // Get all instances
    let instances = sqlx::query!(
        "SELECT id, container_name, assigned_domain FROM app_instances WHERE app_id = $1",
        app.id
    )
    .fetch_all(&state.pool)
    .await?;

    let namespace = format!("hermes-ws-{}", ws_id);
    let instances_clone = instances
        .iter()
        .map(|i| (i.container_name.clone(), i.assigned_domain.clone()))
        .collect::<Vec<_>>();

    // Delete Kubernetes resources asynchronously
    tokio::spawn(async move {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            for (container_name, assigned_domain) in instances_clone {
                if assigned_domain.is_some() {
                    let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &container_name).await;
                }
                let _ = crate::utils::k8s::K8sManager::delete_app(&k8s_client, &namespace, &container_name).await;
                let _ = crate::utils::k8s::K8sManager::delete_knative_service(&k8s_client, &namespace, &container_name).await;
            }
        }
    });

    // Delete associated env variables
    sqlx::query!(
        "DELETE FROM environment_variables WHERE app_instance_id IN (SELECT id FROM app_instances WHERE app_id = $1)",
        app.id
    )
    .execute(&state.pool)
    .await?;

    // Tear down custom domains attached to any of this app's instances (DNS, nginx, ingress + rows).
    for inst in &instances {
        crate::controllers::domain_controller::purge_domains_for_target(&state.pool, ws_id, "app", inst.id).await;
    }

    // BaaS is a standalone project resource now — deleting an app no longer tears down
    // any auth service or its published secret (managed via /baas/:id and project teardown).

    // Delete the application (cascades to app_instances, app_volumes, app_builds, cron_jobs)
    sqlx::query!("DELETE FROM apps WHERE id = $1", app.id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn stream_build_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((app_id, build_id)): Path<(Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // Verify build belongs to app and workspace
    let build = sqlx::query!(
        "SELECT ab.id, ab.status, ab.logs, ab.app_instance_id
         FROM app_builds ab
         JOIN apps a ON ab.app_id = a.id
         WHERE ab.id = $1 AND ab.app_id = $2 AND a.workspace_id = $3",
        build_id, app_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build not found.".to_string()))?;

    let pool = state.pool.clone();
    let instance_id = build.app_instance_id;

    let sse_stream = async_stream::stream! {
        // 1. If build is already completed, stream static logs from DB
        if build.status != "building" {
            for line in build.logs.lines() {
                yield Ok(Event::default().data(line.to_string()));
            }
            return;
        }

        // 2. Build is active, connect to Kubernetes and stream live logs
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                yield Ok(Event::default().data(format!("[System Error] Conexiunea la Kubernetes a eșuat: {}", e)));
                return;
            }
        };

        let namespace = format!("hermes-ws-{}", ws_id);
        let builder_pod_name = format!("hermes-builder-{}", instance_id);
        let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client, &namespace);

        // Așteptăm ca pod-ul de build să fie programat/pornit (max 30 secunde)
        let mut pod_ready = false;
        for _ in 0..15 {
            if let Ok(pod) = pods_api.get(&builder_pod_name).await {
                if pod.status.is_some() {
                    pod_ready = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        if !pod_ready {
            yield Ok(Event::default().data("[System] Se inițializează mediul de compilare...".to_string()));
        }

        // --- ETAPA 1: CLONER ---
        yield Ok(Event::default().data("=========================================\n ETAPA 1: DESCĂRCARE COD (GIT CLONE) (LIVE)\n=========================================\n".to_string()));

        let cloner_params = kube::api::LogParams {
            container: Some("cloner".to_string()),
            follow: true,
            ..Default::default()
        };

        let mut cloner_stream_success = false;
        if let Ok(log_stream) = pods_api.log_stream(&builder_pod_name, &cloner_params).await {
            cloner_stream_success = true;
            use futures_util::io::AsyncBufReadExt;
            let mut lines = log_stream.lines();
            while let Some(line_res) = lines.next().await {
                match line_res {
                    Ok(line) => {
                        yield Ok(Event::default().data(line));
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        }

        if !cloner_stream_success {
            yield Ok(Event::default().data("Pregătire clonare cod...".to_string()));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        // --- ETAPA 2: KANIKO ---
        yield Ok(Event::default().data("\n\n=========================================\n ETAPA 2: CONSTRUIRE IMAGINE (KANIKO) (LIVE)\n=========================================\n".to_string()));

        // Așteptăm ca containerul Kaniko să devină activ (max 120 secunde, util în caz de pulling imagine mare)
        let mut kaniko_active = false;
        for _ in 0..60 {
            if let Ok(pod) = pods_api.get(&builder_pod_name).await {
                if let Some(status) = pod.status {
                    let container_statuses = status.container_statuses.unwrap_or_default();
                    if let Some(kaniko_status) = container_statuses.iter().find(|c| c.name == "kaniko") {
                        if kaniko_status.state.as_ref().map(|s| s.running.is_some() || s.terminated.is_some()).unwrap_or(false) {
                            kaniko_active = true;
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        if !kaniko_active {
            yield Ok(Event::default().data("Se așteaptă pornirea motorului de build (Kaniko)...".to_string()));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }

        let kaniko_params = kube::api::LogParams {
            container: Some("kaniko".to_string()),
            follow: true,
            ..Default::default()
        };

        if let Ok(log_stream) = pods_api.log_stream(&builder_pod_name, &kaniko_params).await {
            use futures_util::io::AsyncBufReadExt;
            let mut lines = log_stream.lines();
            while let Some(line_res) = lines.next().await {
                match line_res {
                    Ok(line) => {
                        yield Ok(Event::default().data(line));
                    }
                    Err(_) => {
                        break;
                    }
                }
            }
        }

        // --- ETAPA 3: DEPLOY & FINALIZARE (LIVE) ---
        // Cloner/Kaniko au fost transmise live mai sus, dar orchestratorul mai
        // scrie log-uri (deploy, clasificare erori, sumar final) în DB DUPĂ ce
        // Kaniko termină. Le transmitem incremental aici, urmărind ce se adaugă în
        // coloana `logs` până când build-ul nu mai e `building` — astfel
        // utilizatorul vede totul fără să dea refresh pe pagină.
        let mut last_len: usize = sqlx::query_scalar!("SELECT logs FROM app_builds WHERE id = $1", build_id)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .map(|l| l.len())
            .unwrap_or(0);
        let mut separator_sent = false;
        // Maxim ~6 minute pentru fazele post-build (240 * 1500ms).
        for _ in 0..240 {
            match sqlx::query!("SELECT status, logs FROM app_builds WHERE id = $1", build_id)
                .fetch_optional(&pool)
                .await
            {
                Ok(Some(row)) => {
                    if row.logs.len() > last_len {
                        if let Some(appended) = row.logs.get(last_len..) {
                            if !separator_sent {
                                yield Ok(Event::default().data("\n=========================================\n ETAPA 3: DEPLOY & FINALIZARE (LIVE)\n=========================================".to_string()));
                                separator_sent = true;
                            }
                            for line in appended.lines() {
                                yield Ok(Event::default().data(line.to_string()));
                            }
                            last_len = row.logs.len();
                        }
                    }
                    if row.status != "building" {
                        yield Ok(Event::default().data(format!("\n--- Build finalizat (status: {}) ---", row.status)));
                        break;
                    }
                }
                _ => break,
            }
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
    };

    Ok(Sse::new(sse_stream))
}

/// Live container logs over a WebSocket (push, no polling on the client side).
/// Authenticated via the standard `?token=` query param like the SSE streams.
pub async fn stream_instance_logs_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(query): Query<LogQuery>,
    Path((_app_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let instance = sqlx::query_as::<_, AppInstance>(
        "SELECT ai.* FROM app_instances ai
         JOIN apps a ON ai.app_id = a.id
         WHERE ai.id = $1 AND a.workspace_id = $2"
    )
    .bind(instance_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application instance not found.".to_string()))?;

    let container_name = instance.container_name.clone();
    let is_previous = query.previous.unwrap_or(false);

    Ok(ws.on_upgrade(move |socket| handle_instance_log_socket(socket, ws_id, container_name, is_previous)))
}

async fn handle_instance_log_socket(
    socket: WebSocket,
    ws_id: Uuid,
    container_name: String,
    is_previous: bool,
) {
    let (mut sender, mut receiver) = socket.split();

    // Pump Kubernetes container logs into the websocket.
    let mut send_task = tokio::spawn(async move {
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                let _ = sender.send(Message::Text(format!("[Console Error] Conexiunea la Kubernetes a eșuat: {}", e))).await;
                return;
            }
        };
        let namespace = format!("hermes-ws-{}", ws_id);
        let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(k8s_client, &namespace);
        let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));

        loop {
            let pod_list = match pods_api.list(&lp).await {
                Ok(list) => list,
                Err(e) => {
                    if sender.send(Message::Text(format!("[Console Error] Eșec la listarea pod-urilor: {}", e))).await.is_err() { break; }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
            };

            let pod = match pod_list.items.first() {
                Some(p) => p,
                None => {
                    if sender.send(Message::Text("[Console] Se așteaptă programarea pod-ului pe nod...".to_string())).await.is_err() { break; }
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

            let phase = pod.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".to_string());
            if phase == "Pending" || phase == "Unknown" {
                if sender.send(Message::Text(format!("[Console] Instanța se inițializează (Stare: {})...", phase))).await.is_err() { break; }
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }

            let log_params = kube::api::LogParams {
                follow: !is_previous && (phase == "Running"),
                previous: is_previous,
                tail_lines: Some(100),
                ..Default::default()
            };

            match pods_api.log_stream(&pod_name, &log_params).await {
                Ok(log_stream) => {
                    if sender.send(Message::Text("[Console] Conexiune stabilă cu containerul. Se preiau logurile:".to_string())).await.is_err() { break; }
                    use futures_util::io::AsyncBufReadExt;
                    let mut lines = log_stream.lines();
                    while let Some(line_res) = lines.next().await {
                        match line_res {
                            Ok(line) => {
                                if sender.send(Message::Text(line)).await.is_err() { return; }
                            }
                            Err(_) => break,
                        }
                    }
                    // Snapshot mode (previous logs) is one-shot.
                    if is_previous { break; }
                    // The follow stream ended (pod restarted/terminated) — wait and reconnect.
                    if sender.send(Message::Text("[Console] Fluxul s-a încheiat. Reconectare...".to_string())).await.is_err() { break; }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
                Err(e) => {
                    if sender.send(Message::Text(format!("[Console Warning] Eroare la fluxul de logs: {}", e))).await.is_err() { break; }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    });

    // Detect client disconnect.
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg { break; }
        }
    });

    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    };
}