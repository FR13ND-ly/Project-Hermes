use axum::{extract::{Path, Query, State}, http::StatusCode, Json};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::domain_dto::{AddDomainRequest, DomainResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::domain_model::{Domain, DomainRoutingType, DomainStatus};
use crate::utils::{cloudflare, error::AppError};
use crate::utils::pagination::{PaginationParams, Paginated};

/// What a domain points at, resolved from its target_type/target_id.
struct ResolvedTarget {
    routing_type: DomainRoutingType,
    nginx_target_host: Option<String>,
    nginx_root_path: Option<String>,
    target_port: i32,
    target_name: Option<String>,
    /// Databases get only a DNS record (TCP) — no nginx site / ingress.
    dns_only: bool,
}

/// Resolve + validate a domain's target within the workspace.
async fn resolve_target(
    pool: &sqlx::PgPool,
    ws_id: Uuid,
    payload: &AddDomainRequest,
) -> Result<ResolvedTarget, AppError> {
    match payload.target_type.as_str() {
        "app" => {
            let id = payload.target_id.ok_or_else(|| AppError::Validation("target_id is required for an app domain.".to_string()))?;
            let row = sqlx::query!(
                "SELECT ai.container_name, ai.internal_port, a.name
                 FROM app_instances ai JOIN apps a ON ai.app_id = a.id
                 WHERE ai.id = $1 AND a.workspace_id = $2",
                id, ws_id
            )
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::NotFound("App instance not found in this workspace.".to_string()))?;
            Ok(ResolvedTarget {
                routing_type: DomainRoutingType::ReverseProxy,
                nginx_target_host: Some(row.container_name),
                nginx_root_path: None,
                target_port: row.internal_port,
                target_name: Some(row.name),
                dns_only: false,
            })
        }
        "serverless" => {
            let id = payload.target_id.ok_or_else(|| AppError::Validation("target_id is required for a serverless domain.".to_string()))?;
            let row = sqlx::query!(
                "SELECT name FROM serverless_instances WHERE id = $1 AND workspace_id = $2",
                id, ws_id
            )
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Serverless instance not found in this workspace.".to_string()))?;
            let svc = format!("fn-{}-proxy-svc", crate::controllers::serverless_controller::slugify(&row.name));
            Ok(ResolvedTarget {
                routing_type: DomainRoutingType::ReverseProxy,
                nginx_target_host: Some(svc),
                nginx_root_path: None,
                target_port: 80,
                target_name: Some(row.name),
                dns_only: false,
            })
        }
        "database" => {
            let id = payload.target_id.ok_or_else(|| AppError::Validation("target_id is required for a database domain.".to_string()))?;
            let row = sqlx::query!(
                "SELECT name, is_external, external_port FROM databases WHERE id = $1 AND workspace_id = $2",
                id, ws_id
            )
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::NotFound("Database not found in this workspace.".to_string()))?;
            if !row.is_external {
                return Err(AppError::Validation("Baza de date trebuie expusă extern (TCP) înainte de a-i atribui un domeniu.".to_string()));
            }
            Ok(ResolvedTarget {
                routing_type: DomainRoutingType::Custom,
                nginx_target_host: None,
                nginx_root_path: None,
                target_port: row.external_port.unwrap_or(0),
                target_name: Some(row.name),
                dns_only: true,
            })
        }
        _ => Ok(ResolvedTarget {
            routing_type: payload.routing_type.unwrap_or(DomainRoutingType::ReverseProxy),
            nginx_target_host: payload.nginx_target_host.clone(),
            nginx_root_path: payload.nginx_root_path.clone(),
            target_port: 80,
            target_name: None,
            dns_only: false,
        }),
    }
}

/// Resolve a display name for a domain's target (for list/detail responses).
async fn target_name_for(pool: &sqlx::PgPool, target_type: &str, target_id: Option<Uuid>) -> Option<String> {
    let id = target_id?;
    match target_type {
        "app" => sqlx::query_scalar!(
            "SELECT a.name FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1", id
        ).fetch_optional(pool).await.ok().flatten(),
        "serverless" => sqlx::query_scalar!(
            "SELECT name FROM serverless_instances WHERE id = $1", id
        ).fetch_optional(pool).await.ok().flatten(),
        "database" => sqlx::query_scalar!(
            "SELECT name FROM databases WHERE id = $1", id
        ).fetch_optional(pool).await.ok().flatten(),
        _ => None,
    }
}

/// Cloudflare/Ingress config resolved from the PROJECT that owns a domain's target.
/// (CF settings moved from workspace to project level — see migration 20260614130000.)
struct ProjectCf {
    api_token: Option<String>,
    zone_id: Option<String>,
    ingress_ip: Option<String>,
}

async fn resolve_project_cf(
    pool: &sqlx::PgPool,
    ws_id: Uuid,
    target_type: &str,
    target_id: Option<Uuid>,
) -> ProjectCf {
    let empty = ProjectCf { api_token: None, zone_id: None, ingress_ip: None };
    let Some(id) = target_id else { return empty; };

    let project_id: Option<Uuid> = match target_type {
        "app" => sqlx::query_scalar!(
            "SELECT a.project_id FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1 AND a.workspace_id = $2",
            id, ws_id
        ).fetch_optional(pool).await.ok().flatten(),
        "serverless" => sqlx::query_scalar!(
            "SELECT project_id FROM serverless_instances WHERE id = $1 AND workspace_id = $2",
            id, ws_id
        ).fetch_optional(pool).await.ok().flatten(),
        "database" => sqlx::query_scalar!(
            "SELECT project_id FROM databases WHERE id = $1 AND workspace_id = $2",
            id, ws_id
        ).fetch_optional(pool).await.ok().flatten(),
        _ => None,
    };

    let Some(pid) = project_id else { return empty; };

    let Some(p) = sqlx::query!(
        "SELECT cloudflare_credential_id, ingress_ip FROM projects WHERE id = $1",
        pid
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten() else { return empty; };

    // Cloudflare config comes from the linked workspace credential (encrypted).
    let mut api_token = None;
    let mut zone_id = None;
    if let Some(cred_id) = p.cloudflare_credential_id {
        if let Ok(Some(c)) = sqlx::query!(
            "SELECT encrypted_token, nonce, zone_id FROM cloudflare_credentials WHERE id = $1",
            cred_id
        )
        .fetch_optional(pool)
        .await
        {
            api_token = crate::utils::crypto::decrypt_env_value(&c.encrypted_token, &c.nonce).ok();
            zone_id = Some(c.zone_id);
        }
    }

    ProjectCf { api_token, zone_id, ingress_ip: p.ingress_ip }
}

fn to_response(d: Domain, target_name: Option<String>) -> DomainResponse {
    DomainResponse {
        id: d.id,
        fqdn: d.fqdn,
        target_type: d.target_type,
        target_id: d.target_id,
        target_name,
        routing_type: d.routing_type,
        status: d.status,
        client_max_body_size: d.client_max_body_size,
        is_ssl: d.is_ssl,
        nginx_config_content: d.nginx_config_content,
        cf_proxy_active: d.cf_proxy_active,
        nginx_target_host: d.nginx_target_host,
        nginx_root_path: d.nginx_root_path,
    }
}

/// Query for `GET /domains`: pagination + an optional `projectId` scope. When
/// `projectId` is set, only domains whose target (app/serverless/database) belongs to
/// that project are returned — `custom` (project-less) domains are excluded. Omitting
/// it keeps the workspace-wide listing. (No serde flatten — serde_urlencoded, which
/// Axum's Query uses, doesn't support it.)
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDomainsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub project_id: Option<Uuid>,
}

pub async fn list_domains(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Query(q): Query<ListDomainsQuery>,
) -> Result<Json<Paginated<DomainResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let (page, page_size, offset) = PaginationParams { page: q.page, page_size: q.page_size }.resolve();
    let project_id = q.project_id;

    // Optional project scope: a domain matches when its target resource lives in the
    // project. `$N::uuid IS NULL` short-circuits to the full workspace list when unset.
    const PROJECT_FILTER: &str = "
        AND ($PID::uuid IS NULL
          OR (target_type = 'app' AND EXISTS (
                SELECT 1 FROM app_instances ai JOIN apps a ON ai.app_id = a.id
                WHERE ai.id = domains.target_id AND a.project_id = $PID))
          OR (target_type = 'serverless' AND EXISTS (
                SELECT 1 FROM serverless_instances s WHERE s.id = domains.target_id AND s.project_id = $PID))
          OR (target_type = 'database' AND EXISTS (
                SELECT 1 FROM databases db WHERE db.id = domains.target_id AND db.project_id = $PID)))";

    let count_sql = format!(
        "SELECT COUNT(*) FROM domains WHERE workspace_id = $1 {}",
        PROJECT_FILTER.replace("$PID", "$2")
    );
    let total: i64 = sqlx::query_scalar::<_, i64>(&count_sql)
        .bind(ws_id)
        .bind(project_id)
        .fetch_one(&state.pool)
        .await?;

    let list_sql = format!(
        "SELECT * FROM domains WHERE workspace_id = $1 {} ORDER BY created_at DESC LIMIT $3 OFFSET $4",
        PROJECT_FILTER.replace("$PID", "$2")
    );
    let domains = sqlx::query_as::<_, Domain>(&list_sql)
        .bind(ws_id)
        .bind(project_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&state.pool)
        .await?;

    let mut items = Vec::with_capacity(domains.len());
    for d in domains {
        let name = target_name_for(&state.pool, &d.target_type, d.target_id).await;
        items.push(to_response(d, name));
    }

    Ok(Json(Paginated::new(items, total, page, page_size)))
}

#[derive(serde::Deserialize)]
pub struct DomainLogsQuery {
    pub lines: Option<usize>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainLogsResponse {
    pub lines: Vec<String>,
    /// True when this route serves traffic through the host nginx (so logs exist).
    pub supported: bool,
}

/// Real nginx access logs for a custom/attached domain, tailed from the per-site
/// access log the nginx template writes to /var/log/nginx/<fqdn>.access.log.
pub async fn get_domain_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(domain_id): Path<Uuid>,
    Query(q): Query<DomainLogsQuery>,
) -> Result<Json<DomainLogsResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let domain = sqlx::query!(
        "SELECT fqdn, target_type FROM domains WHERE id = $1 AND workspace_id = $2",
        domain_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Domain not found in this workspace.".to_string()))?;

    // Databases are TCP (no nginx) → no access logs to tail.
    if domain.target_type == "database" {
        return Ok(Json(DomainLogsResponse { lines: vec![], supported: false }));
    }

    let n = q.lines.unwrap_or(200).min(2000);
    let path = format!("/var/log/nginx/{}.access.log", domain.fqdn);
    let lines = match std::fs::read_to_string(&path) {
        Ok(content) => {
            let all: Vec<&str> = content.lines().collect();
            let start = all.len().saturating_sub(n);
            all[start..].iter().map(|s| s.to_string()).collect()
        }
        Err(_) => vec![],
    };

    Ok(Json(DomainLogsResponse { lines, supported: true }))
}

/// Human-readable summary of the routing actually applied. Edge routing is now a
/// k8s Ingress reconciled by Traefik (with automatic TLS via cert-manager), not a
/// host Nginx site — but we keep ADR-003's transparency goal by storing this in
/// `domains.nginx_config_content` so the UI can show exactly what routes a domain.
fn render_applied_config(fqdn: &str, service: Option<&str>, port: i32, body_size_mb: i32) -> String {
    let tls = match std::env::var("HERMES_SSL_ISSUER") {
        Ok(issuer) if !issuer.trim().is_empty() => format!("tls: automatic (cert-manager issuer: {})", issuer),
        _ => "tls: disabled (no HERMES_SSL_ISSUER configured)".to_string(),
    };
    format!(
        "# Applied via Traefik (Kubernetes Ingress)\n\
         host: {fqdn}\n\
         backend: {svc}:{port}\n\
         {tls}\n\
         maxBodySize: {body_size_mb}MB\n",
        fqdn = fqdn,
        svc = service.unwrap_or("(unknown)"),
        port = port,
        tls = tls,
        body_size_mb = body_size_mb,
    )
}

pub async fn add_domain(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<AddDomainRequest>,
) -> Result<(StatusCode, Json<DomainResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let fqdn = payload.fqdn.to_lowercase().trim().to_string();
    let body_size = payload.client_max_body_size.unwrap_or(50);
    let ssl_enabled = payload.is_ssl.unwrap_or(true);

    let domain_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM domains WHERE workspace_id = $1 AND fqdn = $2)",
        ws_id, fqdn
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if domain_exists {
        return Err(AppError::Conflict("This domain is already registered in this workspace.".to_string()));
    }

    let target = resolve_target(&state.pool, ws_id, &payload).await?;

    // Cloudflare/Ingress config now comes from the target's project, not the workspace.
    let cf = resolve_project_cf(&state.pool, ws_id, &payload.target_type, payload.target_id).await;

    let target_ip = match &cf.ingress_ip {
        Some(ip) if !ip.trim().is_empty() => ip.clone(),
        _ => std::env::var("HERMES_INGRESS_IP").unwrap_or_else(|_| "127.0.0.1".to_string()),
    };

    let mut zone_id = None;
    let mut record_id = None;
    if let (Some(token), Some(z_id)) = (&cf.api_token, &cf.zone_id) {
        if !token.trim().is_empty() && !z_id.trim().is_empty() {
            let (cf_z, cf_r) = cloudflare::create_dns_record(&fqdn, &target_ip, true, Some(token), Some(z_id)).await?;
            zone_id = Some(cf_z);
            record_id = Some(cf_r);
        }
    }

    // Databases are TCP: just a DNS record, no HTTP edge. Everything else is
    // routed by Traefik via the k8s Ingress created below — no host Nginx.
    let final_nginx_content = if target.dns_only {
        None
    } else {
        Some(render_applied_config(
            &fqdn,
            target.nginx_target_host.as_deref(),
            target.target_port,
            body_size,
        ))
    };

    let domain_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO domains (id, workspace_id, fqdn, target_type, target_id, routing_type, client_max_body_size, is_ssl, nginx_target_host, nginx_root_path, nginx_config_content, created_by, status, cloudflare_zone_id, cloudflare_record_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'active'::domain_status, $13, $14)",
        domain_id,
        ws_id,
        fqdn,
        payload.target_type,
        payload.target_id,
        target.routing_type as _,
        body_size,
        ssl_enabled,
        target.nginx_target_host,
        target.nginx_root_path,
        final_nginx_content,
        claims.sub,
        zone_id,
        record_id
    )
    .execute(&state.pool)
    .await?;

    // Wire the K8s ingress for HTTP reverse-proxy targets.
    if !target.dns_only && target.routing_type == DomainRoutingType::ReverseProxy {
        if let Some(ref service_name) = target.nginx_target_host {
            let namespace = format!("hermes-ws-{}", ws_id);
            if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                    &k8s_client,
                    &namespace,
                    &format!("domain-{}", domain_id),
                    &fqdn,
                    service_name,
                    target.target_port,
                ).await;
            }
        }
    }

    // Keep the serverless function aware of its public domain.
    if payload.target_type == "serverless" {
        if let Some(fn_id) = payload.target_id {
            let _ = sqlx::query!("UPDATE serverless_instances SET assigned_domain = $1, updated_at = now() WHERE id = $2", fqdn, fn_id)
                .execute(&state.pool).await;
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(DomainResponse {
            id: domain_id,
            fqdn,
            target_type: payload.target_type,
            target_id: payload.target_id,
            target_name: target.target_name,
            routing_type: target.routing_type,
            status: DomainStatus::Active,
            client_max_body_size: body_size,
            is_ssl: ssl_enabled,
            nginx_config_content: final_nginx_content,
            cf_proxy_active: true,
            nginx_target_host: target.nginx_target_host,
            nginx_root_path: target.nginx_root_path,
        }),
    ))
}

pub async fn verify_and_sync_domain(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(domain_id): Path<Uuid>,
) -> Result<Json<DomainResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    let domain = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE id = $1 AND workspace_id = $2"
    )
    .bind(domain_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Domain not found in this workspace.".to_string()))?;

    let cf = resolve_project_cf(&state.pool, ws_id, &domain.target_type, domain.target_id).await;

    let target_ip = match &cf.ingress_ip {
        Some(ip) if !ip.trim().is_empty() => ip.clone(),
        _ => std::env::var("HERMES_INGRESS_IP").unwrap_or_else(|_| "127.0.0.1".to_string()),
    };
    let (zone_id, record_id) = cloudflare::create_dns_record(
        &domain.fqdn,
        &target_ip,
        domain.cf_proxy_active,
        cf.api_token.as_deref(),
        cf.zone_id.as_deref()
    ).await?;

    let dns_only = domain.target_type == "database";

    let final_nginx_content = if dns_only {
        domain.nginx_config_content.clone()
    } else {
        let mut applied_port = 80;
        if domain.routing_type == DomainRoutingType::ReverseProxy {
            if let Some(ref service_name) = domain.nginx_target_host {
                let namespace = format!("hermes-ws-{}", ws_id);
                let target_port = sqlx::query_scalar!(
                    "SELECT internal_port FROM app_instances WHERE container_name = $1",
                    service_name
                )
                .fetch_optional(&state.pool)
                .await?
                .unwrap_or(80);
                applied_port = target_port;

                if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                    let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                        &k8s_client,
                        &namespace,
                        &format!("domain-{}", domain.id),
                        &domain.fqdn,
                        service_name,
                        target_port,
                    ).await;
                }
            }
        }
        Some(render_applied_config(
            &domain.fqdn,
            domain.nginx_target_host.as_deref(),
            applied_port,
            domain.client_max_body_size,
        ))
    };

    sqlx::query!(
        "UPDATE domains
         SET status = 'active'::domain_status, cloudflare_zone_id = $1, cloudflare_record_id = $2, nginx_config_content = $3, updated_at = now()
         WHERE id = $4",
        zone_id, record_id, final_nginx_content, domain.id
    )
    .execute(&state.pool)
    .await?;

    let name = target_name_for(&state.pool, &domain.target_type, domain.target_id).await;
    let mut d = domain;
    d.status = DomainStatus::Active;
    d.nginx_config_content = final_nginx_content;
    Ok(Json(to_response(d, name)))
}

/// Best-effort teardown of a domain's *external* resources (Cloudflare DNS
/// record, nginx site, k8s ingress). It does NOT delete the DB row — the caller
/// removes that (e.g. inside a transaction). Used when a parent resource/project
/// is deleted so domains don't leave dangling DNS/ingress behind.
pub async fn teardown_domain_resources(pool: &sqlx::PgPool, ws_id: Uuid, domain: &Domain) {
    let cf = resolve_project_cf(pool, ws_id, &domain.target_type, domain.target_id).await;

    if let (Some(zone_id), Some(record_id)) =
        (domain.cloudflare_zone_id.clone(), domain.cloudflare_record_id.clone())
    {
        let _ = cloudflare::delete_dns_record(&zone_id, &record_id, cf.api_token.as_deref()).await;
    }

    // Databases are DNS-only (no ingress / nginx site to remove).
    if domain.target_type != "database" {
        let namespace = format!("hermes-ws-{}", ws_id);
        let domain_res_name = format!("domain-{}", domain.id);
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &domain_res_name).await;
        }
    }
}

/// Tears down and deletes every domain attached to a single resource (matched by
/// `target_type` + `target_id`). Best-effort, used when the underlying resource
/// (app instance, serverless function, database) is deleted so its domains never
/// linger with live DNS/ingress.
pub async fn purge_domains_for_target(pool: &sqlx::PgPool, ws_id: Uuid, target_type: &str, target_id: Uuid) {
    let domains = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE workspace_id = $1 AND target_type = $2 AND target_id = $3"
    )
    .bind(ws_id)
    .bind(target_type)
    .bind(target_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for d in &domains {
        teardown_domain_resources(pool, ws_id, d).await;
    }

    let _ = sqlx::query!(
        "DELETE FROM domains WHERE workspace_id = $1 AND target_type = $2 AND target_id = $3",
        ws_id, target_type, target_id
    )
    .execute(pool)
    .await;
}

pub async fn remove_domain(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(domain_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let domain = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE id = $1 AND workspace_id = $2"
    )
    .bind(domain_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Domain not found in this workspace.".to_string()))?;

    let cf = resolve_project_cf(&state.pool, ws_id, &domain.target_type, domain.target_id).await;

    if let (Some(zone_id), Some(record_id)) = (domain.cloudflare_zone_id.clone(), domain.cloudflare_record_id.clone()) {
        cloudflare::delete_dns_record(&zone_id, &record_id, cf.api_token.as_deref()).await?;
    }

    // Clear the serverless function's domain reference.
    if domain.target_type == "serverless" {
        if let Some(fn_id) = domain.target_id {
            let _ = sqlx::query!("UPDATE serverless_instances SET assigned_domain = NULL, updated_at = now() WHERE id = $1", fn_id)
                .execute(&state.pool).await;
        }
    }

    let dns_only = domain.target_type == "database";
    if !dns_only {
        let namespace = format!("hermes-ws-{}", ws_id);
        let domain_res_name = format!("domain-{}", domain.id);
        tokio::spawn(async move {
            if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &domain_res_name).await;
            }
        });
    }

    sqlx::query!("DELETE FROM domains WHERE id = $1", domain.id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_domain(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(domain_id): Path<Uuid>,
    Json(payload): Json<AddDomainRequest>,
) -> Result<Json<DomainResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let domain = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE id = $1 AND workspace_id = $2"
    )
    .bind(domain_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Domain not found in this workspace.".to_string()))?;

    // DNS-only (database) domains only carry SSL/body-size knobs; nothing to redeploy.
    if domain.target_type == "database" {
        let body_size = payload.client_max_body_size.unwrap_or(domain.client_max_body_size);
        let ssl_enabled = payload.is_ssl.unwrap_or(domain.is_ssl);
        sqlx::query!(
            "UPDATE domains SET client_max_body_size = $1, is_ssl = $2, updated_at = now() WHERE id = $3",
            body_size, ssl_enabled, domain.id
        ).execute(&state.pool).await?;
        let name = target_name_for(&state.pool, &domain.target_type, domain.target_id).await;
        let mut d = domain;
        d.client_max_body_size = body_size;
        d.is_ssl = ssl_enabled;
        return Ok(Json(to_response(d, name)));
    }

    let body_size = payload.client_max_body_size.unwrap_or(domain.client_max_body_size);
    let ssl_enabled = payload.is_ssl.unwrap_or(domain.is_ssl);
    let routing_type = payload.routing_type.unwrap_or(domain.routing_type);

    let mut applied_port = 80;
    if routing_type == DomainRoutingType::ReverseProxy {
        if let Some(ref service_name) = payload.nginx_target_host {
            let namespace = format!("hermes-ws-{}", ws_id);
            let target_port = sqlx::query_scalar!(
                "SELECT internal_port FROM app_instances WHERE container_name = $1",
                service_name
            )
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or(80);
            applied_port = target_port;

            if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                    &k8s_client,
                    &namespace,
                    &format!("domain-{}", domain.id),
                    &domain.fqdn,
                    service_name,
                    target_port,
                ).await;
            }
        }
    } else {
        // Non reverse-proxy: make sure no stale Ingress lingers.
        let namespace = format!("hermes-ws-{}", ws_id);
        let domain_res_name = format!("domain-{}", domain.id);
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let _ = crate::utils::k8s::K8sManager::delete_ingress(&k8s_client, &namespace, &domain_res_name).await;
        }
    }

    let final_nginx_content = render_applied_config(
        &domain.fqdn,
        payload.nginx_target_host.as_deref(),
        applied_port,
        body_size,
    );

    sqlx::query!(
        "UPDATE domains
         SET routing_type = $1, client_max_body_size = $2, is_ssl = $3, nginx_target_host = $4, nginx_root_path = $5, nginx_config_content = $6, updated_at = now()
         WHERE id = $7",
        routing_type as _,
        body_size,
        ssl_enabled,
        payload.nginx_target_host,
        payload.nginx_root_path,
        final_nginx_content,
        domain.id
    )
    .execute(&state.pool)
    .await?;

    let name = target_name_for(&state.pool, &domain.target_type, domain.target_id).await;
    let mut d = domain;
    d.routing_type = routing_type;
    d.client_max_body_size = body_size;
    d.is_ssl = ssl_enabled;
    d.nginx_target_host = payload.nginx_target_host;
    d.nginx_root_path = payload.nginx_root_path;
    d.nginx_config_content = Some(final_nginx_content);
    Ok(Json(to_response(d, name)))
}
