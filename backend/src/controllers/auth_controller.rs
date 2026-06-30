use axum::{extract::{State, Path}, http::{HeaderMap, StatusCode}, Json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use chrono::Utc;

use crate::app_state::AppState;
use crate::dtos::auth_dto::{
    ActivateAccountRequest, AuthTokenResponse, LoginRequest, PasswordChangeRequest, 
    ProvisionUserRequest, RefreshRequest, SwitchWorkspaceRequest, UserResponse
};
use crate::models::user_model::{User, UserStatus};
use crate::utils::{crypto, jwt, error::AppError};

/// Extracts the real client IP, honouring the reverse proxy's forwarding headers.
fn extract_client_ip(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthTokenResponse>, AppError> {
    let client_ip = extract_client_ip(&headers);
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Brute-force throttle, keyed by client IP (falls back to a shared bucket if
    // the proxy didn't forward an IP).
    let rate_key = client_ip.clone().unwrap_or_else(|| "unknown".to_string());
    if !crate::utils::locks::check_rate_limit(&state.pool, &format!("login:{}", rate_key), 10, 300).await {
        return Err(AppError::RateLimited(
            "Too many login attempts. Please try again in a few minutes.".to_string(),
        ));
    }

    let user_opt = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE LOWER(email) = LOWER($1) OR LOWER(username) = LOWER($1)"
    )
    .bind(&payload.login_identity)
    .fetch_optional(&state.pool)
    .await?;

    let user = match user_opt {
        Some(u) => u,
        None => {
            let _ = crate::models::audit_log_model::AuthAuditLog::record(
                &state.pool,
                None,
                &payload.login_identity,
                "LOGIN_FAILED",
                client_ip.clone(),
                user_agent.clone(),
            ).await;
            return Err(AppError::Auth("Invalid credentials.".to_string()));
        }
    };

    let mut user = user;

    if user.status == UserStatus::Suspended {
        let _ = crate::models::audit_log_model::AuthAuditLog::record(
            &state.pool,
            Some(user.id),
            &payload.login_identity,
            "LOGIN_FAILED",
            client_ip.clone(),
            user_agent.clone(),
        ).await;
        return Err(AppError::Auth("This account has been suspended.".to_string()));
    }

    if !crypto::verify_password(&payload.password, &user.password_hash)? {
        let _ = crate::models::audit_log_model::AuthAuditLog::record(
            &state.pool,
            Some(user.id),
            &payload.login_identity,
            "LOGIN_FAILED",
            client_ip.clone(),
            user_agent.clone(),
        ).await;
        return Err(AppError::Auth("Invalid credentials.".to_string()));
    }

    if user.current_workspace_id.is_none() {
        if let Some(member_record) = sqlx::query!(
            "SELECT workspace_id FROM workspace_members WHERE user_id = $1 LIMIT 1",
            user.id
        )
        .fetch_optional(&state.pool)
        .await? {
            user.current_workspace_id = Some(member_record.workspace_id);
            sqlx::query!(
                "UPDATE users SET current_workspace_id = $1 WHERE id = $2",
                user.current_workspace_id,
                user.id
            )
            .execute(&state.pool)
            .await?;
        }
    }

    sqlx::query!(
        "UPDATE users SET last_login_at = now(), last_login_ip = $1, last_login_user_agent = $2 WHERE id = $3",
        client_ip, user_agent, user.id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::models::audit_log_model::AuthAuditLog::record(
        &state.pool,
        Some(user.id),
        &user.email,
        "LOGIN_SUCCESS",
        client_ip.clone(),
        user_agent.clone(),
    ).await;

    let token_bundle = jwt::generate_token_bundle(&user, "")?;
    let token_hash = format!("{:x}", Sha256::digest(token_bundle.refresh_token.as_bytes()));

    sqlx::query!(
        "INSERT INTO refresh_tokens (user_id, token, expires_at) VALUES ($1, $2, $3)",
        user.id, token_hash, token_bundle.refresh_expires_at
    )
    .execute(&state.pool)
    .await?;

    let updated_user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(user.id)
        .fetch_one(&state.pool)
        .await?;

    let expires_in = (token_bundle.refresh_expires_at - Utc::now()).num_seconds();

    Ok(Json(AuthTokenResponse {
        access_token: token_bundle.access_token,
        refresh_token: token_bundle.refresh_token,
        expires_in,
        user: UserResponse::from(updated_user),
    }))
}

pub async fn refresh_session(
    State(state): State<AppState>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<AuthTokenResponse>, AppError> {
    let token_hash = format!("{:x}", Sha256::digest(payload.refresh_token.as_bytes()));

    let token_record = sqlx::query!(
        "DELETE FROM refresh_tokens WHERE token = $1 AND expires_at > now() RETURNING user_id",
        token_hash
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::Auth("Invalid or expired refresh token.".to_string()))?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(token_record.user_id)
        .fetch_one(&state.pool)
        .await?;

    if user.status == UserStatus::Suspended {
        return Err(AppError::Auth("This account has been suspended.".to_string()));
    }

    let token_bundle = jwt::generate_token_bundle(&user, "")?;
    let new_token_hash = format!("{:x}", Sha256::digest(token_bundle.refresh_token.as_bytes()));

    sqlx::query!(
        "INSERT INTO refresh_tokens (user_id, token, expires_at) VALUES ($1, $2, $3)",
        user.id, new_token_hash, token_bundle.refresh_expires_at
    )
    .execute(&state.pool)
    .await?;

    let expires_in = (token_bundle.refresh_expires_at - Utc::now()).num_seconds();

    Ok(Json(AuthTokenResponse {
        access_token: token_bundle.access_token,
        refresh_token: token_bundle.refresh_token,
        expires_in,
        user: UserResponse::from(user),
    }))
}

pub async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Json(payload): Json<PasswordChangeRequest>,
) -> Result<StatusCode, AppError> {
    let client_ip = extract_client_ip(&headers);
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    if !crypto::verify_password(&payload.current_password, &user.password_hash)? {
        return Err(AppError::Auth("Current password inside data is incorrect.".to_string()));
    }

    let hashed_new_password = crypto::hash_password(&payload.new_password)?;

    sqlx::query!(
        "UPDATE users SET password_hash = $1, password_changed_at = now(), updated_at = now() WHERE id = $2",
        hashed_new_password, user.id
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::models::audit_log_model::AuthAuditLog::record(
        &state.pool,
        Some(user.id),
        &user.email,
        "PASSWORD_CHANGE",
        client_ip,
        user_agent,
    ).await;

    Ok(StatusCode::OK)
}

pub async fn provision_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Json(payload): Json<ProvisionUserRequest>,
) -> Result<(StatusCode, Json<String>), AppError> {
    let client_ip = extract_client_ip(&headers);
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let user_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM users WHERE LOWER(email) = LOWER($1) OR LOWER(username) = LOWER($2))",
        payload.email, payload.username
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if user_exists {
        return Err(AppError::Conflict("A user with this email or username already exists.".to_string()));
    }

    let user_id = Uuid::new_v4();
    let temporary_password = Uuid::new_v4().to_string().replace("-", "");
    let hashed_password = crypto::hash_password(&temporary_password)?;

    sqlx::query!(
        "INSERT INTO users (id, username, email, password_hash, is_super_admin, status) VALUES ($1, $2, $3, $4, $5, 'pending_verification'::user_status)",
        user_id, payload.username, payload.email, hashed_password, payload.is_super_admin
    )
    .execute(&state.pool)
    .await?;

    let _ = crate::models::audit_log_model::AuthAuditLog::record(
        &state.pool,
        Some(claims.sub),
        &payload.email,
        "USER_PROVISIONED",
        client_ip,
        user_agent,
    ).await;

    Ok((StatusCode::CREATED, Json(temporary_password)))
}

pub async fn switch_workspace(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Json(payload): Json<SwitchWorkspaceRequest>,
) -> Result<Json<AuthTokenResponse>, AppError> {
    let is_member = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM workspace_members WHERE workspace_id = $1 AND user_id = $2)",
        payload.workspace_id, claims.sub
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !is_member && !claims.is_super_admin {
        return Err(AppError::Permission("You do not have access to this workspace.".to_string()));
    }

    sqlx::query!(
        "UPDATE users SET current_workspace_id = $1 WHERE id = $2",
        payload.workspace_id, claims.sub
    )
    .execute(&state.pool)
    .await?;

    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(claims.sub)
        .fetch_one(&state.pool)
        .await?;

    let token_bundle = jwt::generate_token_bundle(&user, "")?;
    let token_hash = format!("{:x}", Sha256::digest(token_bundle.refresh_token.as_bytes()));

    sqlx::query!(
        "INSERT INTO refresh_tokens (user_id, token, expires_at) VALUES ($1, $2, $3)",
        user.id, token_hash, token_bundle.refresh_expires_at
    )
    .execute(&state.pool)
    .await?;

    let expires_in = (token_bundle.refresh_expires_at - chrono::Utc::now()).num_seconds();

    Ok(Json(AuthTokenResponse {
        access_token: token_bundle.access_token,
        refresh_token: token_bundle.refresh_token,
        expires_in,
        user: user.into(),
    }))
}

pub async fn activate(
    State(state): State<AppState>,
    Json(payload): Json<ActivateAccountRequest>,
) -> Result<StatusCode, AppError> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE LOWER(email) = LOWER($1)")
        .bind(&payload.email)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found.".to_string()))?;

    if user.status != UserStatus::PendingVerification {
        return Err(AppError::Validation("Account is already verified or active.".to_string()));
    }

    if !crypto::verify_password(&payload.temporary_password, &user.password_hash)? {
        return Err(AppError::Auth("Invalid temporary password.".to_string()));
    }

    let hashed_new_password = crypto::hash_password(&payload.new_password)?;

    sqlx::query!(
        "UPDATE users SET password_hash = $1, status = 'active'::user_status, password_changed_at = now(), updated_at = now() WHERE id = $2",
        hashed_new_password, user.id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::OK)
}

pub async fn list_users(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
) -> Result<Json<Vec<UserResponse>>, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Super Admin privileges required".to_string()));
    }

    let users = sqlx::query_as::<_, crate::models::user_model::User>(
        "SELECT * FROM users ORDER BY created_at DESC"
    )
    .fetch_all(&state.pool)
    .await?;

    let response = users.into_iter().map(UserResponse::from).collect();
    Ok(Json(response))
}

pub async fn delete_user(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Path(target_user_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Super Admin privileges required".to_string()));
    }

    if target_user_id == claims.sub {
        return Err(AppError::Validation("You cannot delete your own account.".to_string()));
    }

    let mut tx = state.pool.begin().await?;

    // Reassign user's created workspaces, projects, and domains to the admin performing the deletion
    sqlx::query!(
        "UPDATE workspaces SET created_by = $1 WHERE created_by = $2",
        claims.sub,
        target_user_id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE projects SET created_by = $1 WHERE created_by = $2",
        claims.sub,
        target_user_id
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "UPDATE domains SET created_by = $1 WHERE created_by = $2",
        claims.sub,
        target_user_id
    )
    .execute(&mut *tx)
    .await?;

    let rows_affected = sqlx::query!("DELETE FROM users WHERE id = $1", target_user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

    if rows_affected == 0 {
        tx.rollback().await?;
        return Err(AppError::NotFound("User not found.".to_string()));
    }

    tx.commit().await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn reset_user_password(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Path(target_user_id): Path<Uuid>,
) -> Result<Json<String>, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Super Admin privileges required".to_string()));
    }

    if target_user_id == claims.sub {
        return Err(AppError::Validation("You cannot reset your own password this way. Use the settings page instead.".to_string()));
    }

    // Verify user exists
    let user_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)",
        target_user_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if !user_exists {
        return Err(AppError::NotFound("User not found.".to_string()));
    }

    let temporary_password = Uuid::new_v4().to_string().replace("-", "");
    let hashed_password = crypto::hash_password(&temporary_password)?;

    sqlx::query!(
        "UPDATE users SET password_hash = $1, status = 'pending_verification'::user_status, password_changed_at = now(), updated_at = now() WHERE id = $2",
        hashed_password, target_user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(temporary_password))
}

pub async fn toggle_user_suspend(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(claims): crate::middlewares::auth_middleware::AuthenticatedUser,
    Path(target_user_id): Path<Uuid>,
) -> Result<Json<UserStatus>, AppError> {
    if !claims.is_super_admin {
        return Err(AppError::Permission("Super Admin privileges required".to_string()));
    }

    if target_user_id == claims.sub {
        return Err(AppError::Validation("You cannot suspend your own account.".to_string()));
    }

    let user = sqlx::query_as::<_, crate::models::user_model::User>("SELECT * FROM users WHERE id = $1")
        .bind(target_user_id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found.".to_string()))?;

    let new_status = match user.status {
        UserStatus::Suspended => UserStatus::Active,
        _ => UserStatus::Suspended,
    };

    sqlx::query!(
        "UPDATE users SET status = $1::user_status, updated_at = now() WHERE id = $2",
        new_status as UserStatus, target_user_id
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(new_status))
}

pub async fn get_auth_logs(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(_claims): crate::middlewares::auth_middleware::AuthenticatedUser,
) -> Result<Json<Vec<crate::models::audit_log_model::AuthAuditLog>>, AppError> {
    let logs = sqlx::query_as::<_, crate::models::audit_log_model::AuthAuditLog>(
        "SELECT * FROM auth_audit_logs ORDER BY created_at DESC LIMIT 500"
    )
    .fetch_all(&state.pool)
    .await?;

    Ok(Json(logs))
}

pub async fn get_system_logs(
    State(_state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(_claims): crate::middlewares::auth_middleware::AuthenticatedUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let hostname = std::env::var("HOSTNAME").unwrap_or_else(|_| "local-development".to_string());
    
    // Try to connect to Kubernetes
    let client_res = kube::Client::try_default().await;
    
    match client_res {
        Ok(client) => {
            let system_ns = crate::utils::k8s::K8sManager::system_namespace();
            let pods_api: kube::Api<k8s_openapi::api::core::v1::Pod> = kube::Api::namespaced(client, &system_ns);
            
            let log_params = kube::api::LogParams {
                tail_lines: Some(1000),
                follow: false,
                ..Default::default()
            };
            
            match pods_api.logs(&hostname, &log_params).await {
                Ok(logs) => {
                    Ok(Json(serde_json::json!({ "logs": logs })))
                }
                Err(e) => {
                    // Graceful fallback for local development or permission issues
                    let fallback_msg = format!(
                        "[Hermes] Running in local/mock mode (Pod: {}, Namespace: {}).\nError retrieving logs: {:?}\n\n[Console] Server started on port 5000.\n[DB] PostgreSQL connected.\n[Info] Ready to accept requests.",
                        hostname, system_ns, e
                    );
                    Ok(Json(serde_json::json!({ "logs": fallback_msg })))
                }
            }
        }
        Err(e) => {
            // Local development fallback
            let fallback_msg = format!(
                "[Hermes] Running in local/development mode (Pod: {}).\nKubernetes client not available: {:?}\n\n[Console] Server started on port 5000.\n[DB] PostgreSQL connected.\n[Info] Ready to accept requests.",
                hostname, e
            );
            Ok(Json(serde_json::json!({ "logs": fallback_msg })))
        }
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcRunResponse {
    pub id: uuid::Uuid,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
    pub images_deleted: i32,
    pub builds_pruned: i32,
    pub jobs_pruned: i32,
    pub pods_reaped: i32,
    pub detail: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerResources {
    pub nodes: i32,
    pub cpu_cores_total: f64,
    pub cpu_cores_used: f64,
    pub memory_bytes_total: f64,
    pub memory_bytes_used: f64,
    pub disk_bytes_total: f64,
    pub disk_bytes_used: f64,
    /// True when live usage (kubelet summary) was reachable; false → only totals are real.
    pub usage_available: bool,
}

/// Whole-cluster resource capacity vs. live usage (CPU / RAM / disk), for the header
/// gauges. Totals come from each Node's allocatable; live usage + real disk capacity
/// come from the kubelet `/stats/summary` (proxied via the API server). Best-effort.
pub async fn get_server_resources(
    State(_state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(_claims): crate::middlewares::auth_middleware::AuthenticatedUser,
) -> Result<Json<ServerResources>, AppError> {
    fn parse_cpu_cores(q: &str) -> f64 {
        let q = q.trim();
        if let Some(m) = q.strip_suffix('m') {
            m.trim().parse::<f64>().unwrap_or(0.0) / 1000.0
        } else {
            q.parse::<f64>().unwrap_or(0.0)
        }
    }

    let client = crate::utils::k8s::K8sManager::get_client().await?;
    let nodes_api: kube::Api<k8s_openapi::api::core::v1::Node> = kube::Api::all(client.clone());
    let nodes = nodes_api
        .list(&kube::api::ListParams::default())
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to list nodes: {}", e)))?;

    let mut cpu_total = 0.0;
    let mut mem_total = 0.0;
    let mut disk_total_alloc = 0.0;
    let mut disk_total_summary = 0.0;
    let mut cpu_used = 0.0;
    let mut mem_used = 0.0;
    let mut disk_used = 0.0;
    let mut usage_available = false;
    let node_count = nodes.items.len() as i32;

    for node in &nodes.items {
        let alloc = node.status.as_ref().and_then(|s| s.allocatable.as_ref());
        let cap = node.status.as_ref().and_then(|s| s.capacity.as_ref());
        let pick = |key: &str| -> Option<String> {
            alloc
                .and_then(|m| m.get(key))
                .or_else(|| cap.and_then(|m| m.get(key)))
                .map(|q| q.0.clone())
        };
        if let Some(c) = pick("cpu") {
            cpu_total += parse_cpu_cores(&c);
        }
        if let Some(m) = pick("memory") {
            mem_total += crate::utils::quantity::parse_memory_bytes(&m) as f64;
        }
        if let Some(d) = pick("ephemeral-storage") {
            disk_total_alloc += crate::utils::quantity::parse_memory_bytes(&d) as f64;
        }

        // Live usage (and real fs capacity) from the kubelet summary, proxied through
        // the API server. Best-effort per node so a single unreachable kubelet doesn't
        // fail the whole call.
        let name = match &node.metadata.name {
            Some(n) => n.clone(),
            None => continue,
        };
        let url = format!("/api/v1/nodes/{}/proxy/stats/summary", name);
        if let Ok(req) = axum::http::Request::get(&url).body(Vec::new()) {
            if let Ok(json) = client.request::<serde_json::Value>(req).await {
                if let Some(n) = json.get("node") {
                    if let Some(v) = n.get("cpu").and_then(|c| c.get("usageNanoCores")).and_then(|v| v.as_f64()) {
                        cpu_used += v / 1_000_000_000.0;
                        usage_available = true;
                    }
                    if let Some(v) = n.get("memory").and_then(|c| c.get("workingSetBytes")).and_then(|v| v.as_f64()) {
                        mem_used += v;
                        usage_available = true;
                    }
                    if let Some(fs) = n.get("fs") {
                        if let Some(v) = fs.get("usedBytes").and_then(|v| v.as_f64()) {
                            disk_used += v;
                            usage_available = true;
                        }
                        if let Some(v) = fs.get("capacityBytes").and_then(|v| v.as_f64()) {
                            disk_total_summary += v;
                        }
                    }
                }
            }
        }
    }

    // Prefer the real fs capacity (summary) for disk total; fall back to allocatable
    // ephemeral-storage so totals/used stay consistent.
    let disk_total = if disk_total_summary > 0.0 { disk_total_summary } else { disk_total_alloc };

    Ok(Json(ServerResources {
        nodes: node_count,
        cpu_cores_total: cpu_total,
        cpu_cores_used: cpu_used,
        memory_bytes_total: mem_total,
        memory_bytes_used: mem_used,
        disk_bytes_total: disk_total,
        disk_bytes_used: disk_used,
        usage_available,
    }))
}

/// Recent garbage-collection passes, for the admin console (Logs → GC Worker).
pub async fn get_gc_runs(
    State(state): State<AppState>,
    crate::middlewares::auth_middleware::AuthenticatedUser(_claims): crate::middlewares::auth_middleware::AuthenticatedUser,
) -> Result<Json<Vec<GcRunResponse>>, AppError> {
    let rows = sqlx::query!(
        "SELECT id, started_at, finished_at, status, images_deleted, builds_pruned,
                jobs_pruned, pods_reaped, detail, duration_ms
         FROM gc_runs ORDER BY started_at DESC LIMIT 50"
    )
    .fetch_all(&state.pool)
    .await?;

    let out = rows
        .into_iter()
        .map(|r| GcRunResponse {
            id: r.id,
            started_at: r.started_at,
            finished_at: r.finished_at,
            status: r.status,
            images_deleted: r.images_deleted,
            builds_pruned: r.builds_pruned,
            jobs_pruned: r.jobs_pruned,
            pods_reaped: r.pods_reaped,
            detail: r.detail,
            duration_ms: r.duration_ms,
        })
        .collect();

    Ok(Json(out))
}