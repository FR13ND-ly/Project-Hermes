use axum::{extract::{Path, State}, http::StatusCode, Json};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::dtos::domain_dto::{AddDomainRequest, DomainResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::models::domain_model::{Domain, DomainRoutingType, DomainStatus};
use crate::utils::{cloudflare, nginx::NginxManager, error::AppError};

pub async fn list_domains(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<DomainResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let domains = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE workspace_id = $1 ORDER BY created_at DESC"
    )
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    let response = domains
        .into_iter()
        .map(|d| DomainResponse {
            id: d.id,
            fqdn: d.fqdn,
            routing_type: d.routing_type,
            status: d.status,
            client_max_body_size: d.client_max_body_size,
            is_ssl: d.is_ssl,
            nginx_config_content: d.nginx_config_content,
            cf_proxy_active: d.cf_proxy_active,
            nginx_target_host: d.nginx_target_host,
            nginx_root_path: d.nginx_root_path,
        })
        .collect();

    Ok(Json(response))
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

    let workspace = sqlx::query!(
        "SELECT cloudflare_api_token, cloudflare_zone_id, ingress_ip FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    let target_ip = match &workspace.ingress_ip {
        Some(ip) if !ip.trim().is_empty() => ip.clone(),
        _ => std::env::var("HERMES_INGRESS_IP").unwrap_or_else(|_| "127.0.0.1".to_string()),
    };

    let mut zone_id = None;
    let mut record_id = None;

    if let (Some(token), Some(z_id)) = (&workspace.cloudflare_api_token, &workspace.cloudflare_zone_id) {
        if !token.trim().is_empty() && !z_id.trim().is_empty() {
            let (cf_z, cf_r) = cloudflare::create_dns_record(
                &fqdn, 
                &target_ip, 
                true, // cf_proxy_active
                Some(token),
                Some(z_id)
            ).await?;
            zone_id = Some(cf_z);
            record_id = Some(cf_r);
        }
    }

    let routing_type_str = match payload.routing_type {
        DomainRoutingType::ReverseProxy => "reverse_proxy",
        DomainRoutingType::StaticHost => "static_host",
        DomainRoutingType::Custom => "custom",
    };

    let cert_path = format!("/etc/ssl/hermes/{}.crt", fqdn);
    let key_path = format!("/etc/ssl/hermes/{}.key", fqdn);

    let final_nginx_content = NginxManager::deploy_site(
        routing_type_str,
        &fqdn,
        payload.nginx_target_host.as_deref(),
        payload.nginx_root_path.as_deref(),
        body_size,
        ssl_enabled,
        &cert_path,
        &key_path,
        payload.nginx_config_content.as_deref()
    )?;

    let domain_id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO domains (id, workspace_id, fqdn, routing_type, client_max_body_size, is_ssl, nginx_target_host, nginx_root_path, nginx_config_content, created_by, status, cloudflare_zone_id, cloudflare_record_id) 
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'active'::domain_status, $11, $12)",
        domain_id,
        ws_id,
        fqdn,
        payload.routing_type as _,
        body_size,
        ssl_enabled,
        payload.nginx_target_host,
        payload.nginx_root_path,
        final_nginx_content,
        claims.sub,
        zone_id,
        record_id
    )
    .execute(&state.pool)
    .await?;

    if payload.routing_type == DomainRoutingType::ReverseProxy {
        if let Some(ref service_name) = payload.nginx_target_host {
            let namespace = format!("hermes-ws-{}", ws_id);
            let target_port = sqlx::query_scalar!(
                "SELECT internal_port FROM app_instances WHERE container_name = $1",
                service_name
            )
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or(80);

            if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                let _ = crate::utils::k8s::K8sManager::deploy_ingress(
                    &k8s_client,
                    &namespace,
                    &format!("domain-{}", domain_id),
                    &fqdn,
                    service_name,
                    target_port,
                ).await;
            }
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(DomainResponse {
            id: domain_id,
            fqdn,
            routing_type: payload.routing_type,
            status: DomainStatus::Active,
            client_max_body_size: body_size,
            is_ssl: ssl_enabled,
            nginx_config_content: Some(final_nginx_content),
            cf_proxy_active: true,
            nginx_target_host: payload.nginx_target_host,
            nginx_root_path: payload.nginx_root_path,
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
    
    let workspace = sqlx::query!(
        "SELECT cloudflare_api_token, cloudflare_zone_id, ingress_ip FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    let target_ip = match &workspace.ingress_ip {
        Some(ip) if !ip.trim().is_empty() => ip.clone(),
        _ => std::env::var("HERMES_INGRESS_IP").unwrap_or_else(|_| "127.0.0.1".to_string()),
    };
    let (zone_id, record_id) = cloudflare::create_dns_record(
        &domain.fqdn, 
        &target_ip, 
        domain.cf_proxy_active,
        workspace.cloudflare_api_token.as_deref(),
        workspace.cloudflare_zone_id.as_deref()
    ).await?;

    let routing_type_str = match domain.routing_type {
        DomainRoutingType::ReverseProxy => "reverse_proxy",
        DomainRoutingType::StaticHost => "static_host",
        DomainRoutingType::Custom => "custom",
    };

    let cert_path = format!("/etc/ssl/hermes/{}.crt", domain.fqdn);
    let key_path = format!("/etc/ssl/hermes/{}.key", domain.fqdn);

    let final_nginx_content = NginxManager::deploy_site(
        routing_type_str,
        &domain.fqdn,
        domain.nginx_target_host.as_deref(),
        domain.nginx_root_path.as_deref(),
        domain.client_max_body_size,
        domain.is_ssl,
        &cert_path,
        &key_path,
        domain.nginx_config_content.as_deref()
    )?;

    // Provision Ingress in Kubernetes if it's a reverse proxy mapping
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

    sqlx::query!(
        "UPDATE domains 
         SET status = 'active'::domain_status, 
             cloudflare_zone_id = $1, 
             cloudflare_record_id = $2, 
             nginx_config_content = $3,
             updated_at = now() 
         WHERE id = $4",
        zone_id, 
        record_id, 
        final_nginx_content, 
        domain.id
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(DomainResponse {
        id: domain.id,
        fqdn: domain.fqdn,
        routing_type: domain.routing_type,
        status: DomainStatus::Active,
        client_max_body_size: domain.client_max_body_size,
        is_ssl: domain.is_ssl,
        nginx_config_content: Some(final_nginx_content),
        cf_proxy_active: domain.cf_proxy_active,
        nginx_target_host: domain.nginx_target_host,
        nginx_root_path: domain.nginx_root_path,
    }))
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

    let workspace = sqlx::query!(
        "SELECT cloudflare_api_token FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;

    if let (Some(zone_id), Some(record_id)) = (domain.cloudflare_zone_id.clone(), domain.cloudflare_record_id.clone()) {
        cloudflare::delete_dns_record(&zone_id, &record_id, workspace.cloudflare_api_token.as_deref()).await?;
    }

    // Clean up Ingress in Kubernetes
    let namespace = format!("hermes-ws-{}", ws_id);
    let domain_res_name = format!("domain-{}", domain.id);
    tokio::spawn(async move {
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let _ = crate::utils::k8s::K8sManager::delete_ingress(
                &k8s_client,
                &namespace,
                &domain_res_name,
            ).await;
        }
    });

    NginxManager::delete_site(&domain.fqdn)?;

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

    let mut domain = sqlx::query_as::<_, Domain>(
        "SELECT * FROM domains WHERE id = $1 AND workspace_id = $2"
    )
    .bind(domain_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Domain not found in this workspace.".to_string()))?;

    let body_size = payload.client_max_body_size.unwrap_or(domain.client_max_body_size);
    let ssl_enabled = payload.is_ssl.unwrap_or(domain.is_ssl);

    let routing_type_str = match payload.routing_type {
        DomainRoutingType::ReverseProxy => "reverse_proxy",
        DomainRoutingType::StaticHost => "static_host",
        DomainRoutingType::Custom => "custom",
    };

    let cert_path = format!("/etc/ssl/hermes/{}.crt", domain.fqdn);
    let key_path = format!("/etc/ssl/hermes/{}.key", domain.fqdn);

    let final_nginx_content = NginxManager::deploy_site(
        routing_type_str,
        &domain.fqdn,
        payload.nginx_target_host.as_deref(),
        payload.nginx_root_path.as_deref(),
        body_size,
        ssl_enabled,
        &cert_path,
        &key_path,
        payload.nginx_config_content.as_deref()
    )?;

    // Handle Ingress update in Kubernetes if target host changed
    if payload.routing_type == DomainRoutingType::ReverseProxy {
        if let Some(ref service_name) = payload.nginx_target_host {
            let namespace = format!("hermes-ws-{}", ws_id);
            let target_port = sqlx::query_scalar!(
                "SELECT internal_port FROM app_instances WHERE container_name = $1",
                service_name
            )
            .fetch_optional(&state.pool)
            .await?
            .unwrap_or(80);

            if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                // Redeploy ingress
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
        // If changing away from reverse proxy, clean up ingress
        let namespace = format!("hermes-ws-{}", ws_id);
        let domain_res_name = format!("domain-{}", domain.id);
        if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
            let _ = crate::utils::k8s::K8sManager::delete_ingress(
                &k8s_client,
                &namespace,
                &domain_res_name,
            ).await;
        }
    }

    sqlx::query!(
        "UPDATE domains 
         SET routing_type = $1, client_max_body_size = $2, is_ssl = $3, 
             nginx_target_host = $4, nginx_root_path = $5, nginx_config_content = $6,
             updated_at = now() 
         WHERE id = $7",
        payload.routing_type as _,
        body_size,
        ssl_enabled,
        payload.nginx_target_host,
        payload.nginx_root_path,
        final_nginx_content,
        domain.id
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(DomainResponse {
        id: domain.id,
        fqdn: domain.fqdn,
        routing_type: payload.routing_type,
        status: domain.status,
        client_max_body_size: body_size,
        is_ssl: ssl_enabled,
        nginx_config_content: Some(final_nginx_content),
        cf_proxy_active: domain.cf_proxy_active,
        nginx_target_host: payload.nginx_target_host,
        nginx_root_path: payload.nginx_root_path,
    }))
}