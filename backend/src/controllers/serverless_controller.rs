use axum::{
    extract::{State, Path, Query},
    http::StatusCode,
    response::sse::{Event, Sse},
    Json,
};
use futures_util::stream::Stream;
use futures_util::StreamExt;
use std::convert::Infallible;
use uuid::Uuid;
use chrono::Utc;
use serde_json::json;
use kube::{api::{ListParams, PostParams, DeleteParams, Patch, PatchParams}, Api, Client};
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Service};
use std::time::Instant;

use crate::app_state::AppState;
use crate::models::serverless_model::{ServerlessInstance, ServerlessRoute, ServerlessBuild, ServerlessEnvVariable};
use crate::dtos::serverless_dto::{
    CreateInstanceRequest, UpdateInstanceRequest, InstanceResponse, RouteResponse,
    CreateRouteRequest, UpdateRouteRequest, SetInstanceEnvRequest, InstanceEnvResponse,
    ServerlessBuildResponse,
};
use crate::dtos::env_variable_dto::{ProjectEnvResponse, LinkProjectEnvRequest};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{error::AppError, k8s::K8sManager};
use crate::utils::pagination::{PaginationParams, Paginated};
use crate::controllers::env_variable_controller::clean_env_key;

pub fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

fn to_route_response(r: ServerlessRoute) -> RouteResponse {
    RouteResponse {
        id: r.id,
        instance_id: r.instance_id,
        method: r.method,
        route_path: r.route_path,
        code: r.code,
    }
}

async fn load_instance_response(pool: &sqlx::PgPool, inst: ServerlessInstance) -> InstanceResponse {
    let routes = sqlx::query_as::<_, ServerlessRoute>(
        "SELECT * FROM serverless_routes WHERE instance_id = $1 ORDER BY route_path ASC, method ASC"
    )
    .bind(inst.id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    InstanceResponse {
        id: inst.id,
        workspace_id: inst.workspace_id,
        project_id: inst.project_id,
        name: inst.name,
        runtime: inst.runtime,
        memory_limit_mb: inst.memory_limit_mb,
        status: inst.status,
        assigned_domain: inst.assigned_domain,
        external_port: inst.external_port,
        inherit_project_envs: inst.inherit_project_envs,
        routes: routes.into_iter().map(to_route_response).collect(),
        created_at: inst.created_at,
        updated_at: inst.updated_at,
    }
}

/// Re-load an instance and broadcast its updated state to the dashboard.
async fn broadcast_instance(pool: &sqlx::PgPool, ws_id: Uuid, instance_id: Uuid) {
    if let Ok(inst) = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1")
        .bind(instance_id)
        .fetch_one(pool)
        .await
    {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, instance: inst }
        );
    }
}

/// Verify an instance belongs to the given project within the caller's workspace.
async fn authorize_instance(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    instance_id: Uuid,
    ws_id: Uuid,
) -> Result<(), AppError> {
    let ok = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM serverless_instances WHERE id = $1 AND project_id = $2 AND workspace_id = $3)",
        instance_id, project_id, ws_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !ok {
        return Err(AppError::NotFound("Instanță serverless negăsită în acest proiect.".to_string()));
    }
    Ok(())
}

// ============================ INSTANCE CRUD ============================

pub async fn list_instances(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<InstanceResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM serverless_instances WHERE workspace_id = $1 AND project_id = $2",
        ws_id, project_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let instances = sqlx::query_as::<_, ServerlessInstance>(
        "SELECT * FROM serverless_instances WHERE workspace_id = $1 AND project_id = $2 ORDER BY name ASC LIMIT $3 OFFSET $4"
    )
    .bind(ws_id)
    .bind(project_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let mut items = Vec::new();
    for inst in instances {
        items.push(load_instance_response(&state.pool, inst).await);
    }
    Ok(Json(Paginated::new(items, total, page, page_size)))
}

pub async fn create_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateInstanceRequest>,
) -> Result<(StatusCode, Json<InstanceResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    // Serialize quota-sensitive mutations per workspace (atomic check + insert).
    let _ws_guard = crate::utils::locks::acquire_workspace_lock(&state.pool, ws_id).await?;

    let name = payload.name.trim().to_string();
    if slugify(&name).is_empty() {
        return Err(AppError::Validation("Numele instanței este invalid.".to_string()));
    }
    let runtime = payload.runtime.clone().unwrap_or_else(|| "nodejs-cjs".to_string());
    let memory = payload.memory_limit_mb.unwrap_or(0); // 0 = unlimited (no forced default)

    crate::utils::limits::check_workspace_memory_limit(&state.pool, ws_id, memory as i64, None).await?;

    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO serverless_instances (id, workspace_id, project_id, name, runtime, memory_limit_mb, status, inherit_project_envs)
         VALUES ($1, $2, $3, $4, $5, $6, 'draft', false)",
        id, ws_id, project_id, name, runtime, memory
    )
    .execute(&state.pool)
    .await?;

    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    broadcast_instance(&state.pool, ws_id, id).await;
    Ok((StatusCode::CREATED, Json(load_instance_response(&state.pool, inst).await)))
}

pub async fn get_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1 AND workspace_id = $2")
        .bind(id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Instanță serverless negăsită.".to_string()))?;
    Ok(Json(load_instance_response(&state.pool, inst).await))
}

pub async fn update_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateInstanceRequest>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    // Serialize quota-sensitive mutations per workspace (atomic check + update).
    let _ws_guard = crate::utils::locks::acquire_workspace_lock(&state.pool, ws_id).await?;

    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1 AND workspace_id = $2")
        .bind(id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Instanță serverless negăsită.".to_string()))?;

    let name = payload.name.unwrap_or(inst.name);
    let runtime = payload.runtime.unwrap_or(inst.runtime);
    let memory = payload.memory_limit_mb.unwrap_or(inst.memory_limit_mb);
    let inherit = payload.inherit_project_envs.unwrap_or(inst.inherit_project_envs);

    if payload.memory_limit_mb.is_some() {
        crate::utils::limits::check_workspace_memory_limit(&state.pool, ws_id, memory as i64, Some(id)).await?;
    }

    let assigned_domain = match payload.assigned_domain.clone() {
        Some(opt) => opt,
        None => inst.assigned_domain.clone(),
    };

    // Domain attachment (instance-level): mirror the resource-oriented domain flow.
    if let Some(ref domain_opt) = payload.assigned_domain {
        if let Some(fqdn) = domain_opt {
            let fqdn_clean = fqdn.to_lowercase().trim().to_string();
            if !fqdn_clean.is_empty() {
                let domain_exists = sqlx::query_scalar!(
                    "SELECT EXISTS(SELECT 1 FROM domains WHERE workspace_id = $1 AND fqdn = $2)",
                    ws_id, fqdn_clean
                ).fetch_one(&state.pool).await?.unwrap_or(false);
                if !domain_exists {
                    return Err(AppError::Validation("Domeniul selectat nu este înregistrat în acest workspace.".to_string()));
                }
                let svc_name = format!("fn-{}-proxy-svc", slugify(&name));
                let _ = sqlx::query!(
                    "UPDATE domains SET target_type = 'custom', target_id = NULL, nginx_target_host = NULL
                     WHERE workspace_id = $1 AND target_type = 'serverless' AND target_id = $2 AND fqdn != $3",
                    ws_id, id, fqdn_clean
                ).execute(&state.pool).await;
                let _ = sqlx::query!(
                    "UPDATE domains SET target_type = 'serverless', target_id = $1, routing_type = 'reverse_proxy', nginx_target_host = $2, nginx_root_path = NULL, nginx_config_content = NULL
                     WHERE workspace_id = $3 AND fqdn = $4",
                    id, svc_name, ws_id, fqdn_clean
                ).execute(&state.pool).await;
            }
        } else {
            let _ = sqlx::query!(
                "UPDATE domains SET target_type = 'custom', target_id = NULL, nginx_target_host = NULL
                 WHERE workspace_id = $1 AND target_type = 'serverless' AND target_id = $2",
                ws_id, id
            ).execute(&state.pool).await;
        }
    }

    // Keep domains' target host in sync if the name changed.
    let svc_name = format!("fn-{}-proxy-svc", slugify(&name));
    let _ = sqlx::query!(
        "UPDATE domains SET nginx_target_host = $1 WHERE workspace_id = $2 AND target_type = 'serverless' AND target_id = $3",
        svc_name, ws_id, id
    ).execute(&state.pool).await;

    sqlx::query!(
        "UPDATE serverless_instances SET name = $1, runtime = $2, memory_limit_mb = $3, assigned_domain = $4, inherit_project_envs = $5, status = 'draft', updated_at = now()
         WHERE id = $6",
        name, runtime, memory, assigned_domain, inherit, id
    )
    .execute(&state.pool)
    .await?;

    let updated = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;

    // Publish the instance's public base URL into the project pool once a domain is set.
    let suggested_key = format!("{}_FUNCTION_URL", crate::utils::app_env::sanitize_key_fragment(&updated.name, "FUNCTION"));
    if let Some(domain) = updated.assigned_domain.as_ref().filter(|d| !d.trim().is_empty()) {
        let url = format!("https://{}", domain.trim());
        let _ = crate::utils::app_env::publish_project_env(
            &state.pool, ws_id, updated.project_id, &suggested_key, &url, false, "serverless", id
        ).await;
        if let Ok(insts) = sqlx::query_scalar!(
            "SELECT ael.app_instance_id FROM app_env_links ael
             JOIN project_env_variables pev ON pev.id = ael.project_env_id
             WHERE pev.source = 'serverless' AND pev.source_id = $1",
            id
        ).fetch_all(&state.pool).await {
            for inst in insts {
                crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
            }
        }
    }

    broadcast_instance(&state.pool, ws_id, id).await;
    Ok(Json(load_instance_response(&state.pool, updated).await))
}

pub async fn delete_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1 AND workspace_id = $2")
        .bind(id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Instanță serverless negăsită.".to_string()))?;

    let linked = crate::utils::app_env::unpublish_project_env(&state.pool, "serverless", id).await;
    for inst_id in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst_id);
    }

    // Tear down any custom domains attached to this function (DNS, nginx, ingress + row).
    crate::controllers::domain_controller::purge_domains_for_target(&state.pool, ws_id, "serverless", id).await;

    sqlx::query!("DELETE FROM serverless_instances WHERE id = $1", id).execute(&state.pool).await?;

    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&inst.name));
    tokio::spawn(async move {
        let _ = K8sManager::delete_knative_service(&k8s_client, &namespace, &k8s_svc_name).await;
        let _ = K8sManager::delete_ingress(&k8s_client, &namespace, &k8s_svc_name).await;
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = configmaps.delete(&format!("{}-proxy-config", k8s_svc_name), &DeleteParams::default()).await;
        let deployments: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = deployments.delete(&format!("{}-proxy", k8s_svc_name), &DeleteParams::default()).await;
        let services: Api<Service> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = services.delete(&format!("{}-external", k8s_svc_name), &DeleteParams::default()).await;
        let _ = services.delete(&format!("{}-proxy-svc", k8s_svc_name), &DeleteParams::default()).await;
    });

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionDeleted { workspace_id: ws_id, instance_id: id }
    );
    Ok(StatusCode::NO_CONTENT)
}

// ============================ ROUTE CRUD ============================

fn default_route_code(runtime: &str) -> String {
    if runtime == "nodejs-esm" {
        "export default async function(req, res) {\n    res.status(200).json({ success: true, message: \"Hello from this route!\" });\n};".to_string()
    } else if runtime == "python" {
        "def handler(request):\n    return { \"success\": True, \"message\": \"Hello from this route!\" }".to_string()
    } else {
        "module.exports = async (req, res) => {\n    res.status(200).json({ success: true, message: \"Hello from this route!\" });\n};".to_string()
    }
}

fn norm_path(p: &str) -> String {
    let p = p.trim();
    if p.starts_with('/') { p.to_string() } else { format!("/{}", p) }
}

pub async fn list_routes(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<RouteResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    let routes = sqlx::query_as::<_, ServerlessRoute>(
        "SELECT * FROM serverless_routes WHERE instance_id = $1 ORDER BY route_path ASC, method ASC"
    )
    .bind(instance_id)
    .fetch_all(&state.pool)
    .await?;
    Ok(Json(routes.into_iter().map(to_route_response).collect()))
}

pub async fn create_route(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<CreateRouteRequest>,
) -> Result<(StatusCode, Json<RouteResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let runtime = sqlx::query_scalar!("SELECT runtime FROM serverless_instances WHERE id = $1", instance_id)
        .fetch_one(&state.pool).await?;
    let method = payload.method.trim().to_uppercase();
    let route_path = norm_path(&payload.route_path);
    let code = payload.code.unwrap_or_else(|| default_route_code(&runtime));

    let id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO serverless_routes (id, instance_id, method, route_path, code) VALUES ($1, $2, $3, $4, $5)",
        id, instance_id, method, route_path, code
    )
    .execute(&state.pool)
    .await?;
    // A new route means the running image is stale.
    let _ = sqlx::query!("UPDATE serverless_instances SET status = 'draft', updated_at = now() WHERE id = $1", instance_id)
        .execute(&state.pool).await;

    let r = sqlx::query_as::<_, ServerlessRoute>("SELECT * FROM serverless_routes WHERE id = $1")
        .bind(id).fetch_one(&state.pool).await?;
    Ok((StatusCode::CREATED, Json(to_route_response(r))))
}

pub async fn update_route(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id, route_id)): Path<(Uuid, Uuid, Uuid)>,
    Json(payload): Json<UpdateRouteRequest>,
) -> Result<Json<RouteResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let route = sqlx::query_as::<_, ServerlessRoute>("SELECT * FROM serverless_routes WHERE id = $1 AND instance_id = $2")
        .bind(route_id).bind(instance_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Rută negăsită.".to_string()))?;

    let method = payload.method.map(|m| m.trim().to_uppercase()).unwrap_or(route.method);
    let route_path = payload.route_path.map(|p| norm_path(&p)).unwrap_or(route.route_path);
    let code = payload.code.unwrap_or(route.code);

    sqlx::query!(
        "UPDATE serverless_routes SET method = $1, route_path = $2, code = $3, updated_at = now() WHERE id = $4",
        method, route_path, code, route_id
    )
    .execute(&state.pool)
    .await?;
    let _ = sqlx::query!("UPDATE serverless_instances SET status = 'draft', updated_at = now() WHERE id = $1", instance_id)
        .execute(&state.pool).await;

    let r = sqlx::query_as::<_, ServerlessRoute>("SELECT * FROM serverless_routes WHERE id = $1")
        .bind(route_id).fetch_one(&state.pool).await?;
    Ok(Json(to_route_response(r)))
}

pub async fn delete_route(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id, route_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let affected = sqlx::query!("DELETE FROM serverless_routes WHERE id = $1 AND instance_id = $2", route_id, instance_id)
        .execute(&state.pool).await?.rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound("Rută negăsită.".to_string()));
    }
    let _ = sqlx::query!("UPDATE serverless_instances SET status = 'draft', updated_at = now() WHERE id = $1", instance_id)
        .execute(&state.pool).await;
    Ok(StatusCode::NO_CONTENT)
}

// ============================ DEPLOY ============================

/// Generate the Node wrapper that registers every route by method+path.
fn generate_node_index(routes: &[ServerlessRoute], esm: bool) -> String {
    let mut imports = String::new();
    let mut registrations = String::new();
    for (i, r) in routes.iter().enumerate() {
        let var = format!("h{}", i);
        if esm {
            imports.push_str(&format!("import {} from './route_{}.js';\n", var, i));
        } else {
            imports.push_str(&format!("const {} = require('./route_{}.js');\n", var, i));
        }
        let path_lit = serde_json::to_string(&r.route_path).unwrap_or_else(|_| "\"/\"".to_string());
        let m = r.method.to_lowercase();
        let express_m = if m == "any" { "all".to_string() } else { m };
        registrations.push_str(&format!("app.{}({}, wrap({}));\n", express_m, path_lit, var));
    }

    let head = if esm {
        "import express from 'express';\n".to_string()
    } else {
        "const express = require('express');\n".to_string()
    };

    format!(
        r#"{head}{imports}
const app = express();
const port = process.env.PORT || 8080;
app.use(express.json());
app.use(express.urlencoded({{ extended: true }}));

function wrap(handler) {{
    return async (req, res) => {{
        try {{
            await handler(req, res);
        }} catch (err) {{
            console.error('Handler error:', err);
            if (!res.headersSent) {{
                res.status(500).json({{ error: err.message || 'Internal Server Error' }});
            }}
        }}
    }};
}}

{registrations}
app.use((req, res) => res.status(404).json({{ error: 'Not Found' }}));

app.listen(port, () => {{
    console.log(`Serverless instance listening on port ${{port}}`);
}});
"#,
        head = head, imports = imports, registrations = registrations
    )
}

/// Generate the Python/Flask wrapper that registers every route by method+path.
fn generate_python_index(routes: &[ServerlessRoute]) -> String {
    let mut imports = String::new();
    let mut registrations = String::new();
    for (i, r) in routes.iter().enumerate() {
        imports.push_str(&format!("import route_{}\n", i));
        let path_lit = serde_json::to_string(&r.route_path).unwrap_or_else(|_| "\"/\"".to_string());
        let methods = if r.method.to_uppercase() == "ANY" {
            "['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'OPTIONS']".to_string()
        } else {
            format!("['{}']", r.method.to_uppercase())
        };
        registrations.push_str(&format!(
            "app.add_url_rule({}, 'r{}', make_view(route_{}.handler), methods={})\n",
            path_lit, i, i, methods
        ));
    }

    format!(
        r#"import os
import sys
from flask import Flask, request, jsonify

app = Flask(__name__)
sys.path.append(os.path.dirname(os.path.abspath(__file__)))

{imports}
def make_view(fn):
    def view(*args, **kwargs):
        try:
            res = fn(request)
            if isinstance(res, (dict, list)):
                return jsonify(res)
            return res
        except Exception as err:
            return jsonify({{"error": str(err)}}), 500
    return view

{registrations}
if __name__ == '__main__':
    port = int(os.environ.get('PORT', 8080))
    app.run(host='0.0.0.0', port=port)
"#,
        imports = imports, registrations = registrations
    )
}

pub async fn deploy_instance(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;

    let instance = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1 AND workspace_id = $2")
        .bind(id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Instanță serverless negăsită.".to_string()))?;

    let routes = sqlx::query_as::<_, ServerlessRoute>("SELECT * FROM serverless_routes WHERE instance_id = $1 ORDER BY route_path ASC, method ASC")
        .bind(id).fetch_all(&state.pool).await?;
    if routes.is_empty() {
        return Err(AppError::Validation("Adaugă cel puțin o rută înainte de a lansa instanța.".to_string()));
    }

    let external_port = match instance.external_port {
        Some(p) => p,
        None => {
            let port = get_random_available_port(&state.pool).await?;
            sqlx::query!("UPDATE serverless_instances SET external_port = $1 WHERE id = $2", port, id)
                .execute(&state.pool).await?;
            port
        }
    };

    sqlx::query!("UPDATE serverless_instances SET status = 'building', updated_at = now() WHERE id = $1", id)
        .execute(&state.pool).await?;
    broadcast_instance(&state.pool, ws_id, id).await;

    let build_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO serverless_builds (id, instance_id, workspace_id, status) VALUES ($1, $2, $3, 'building')",
        build_id, id, ws_id
    )
    .execute(&state.pool)
    .await?;

    let pool = state.pool.clone();
    let instance_id = instance.id;
    let instance_name = instance.name.clone();
    let memory_limit_mb = instance.memory_limit_mb;
    let assigned_domain = instance.assigned_domain.clone();
    let runtime = instance.runtime.clone();
    let inherit_project_envs = instance.inherit_project_envs;
    let project_id = instance.project_id;

    tokio::spawn(async move {
        let start_time = Instant::now();
        let k8s_client = match K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => { let _ = save_build_error(&pool, instance_id, build_id, &format!("Eșec conexiune Kubernetes: {}", e)).await; return; }
        };

        let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb, max_cpu_millicores FROM workspaces WHERE id = $1", ws_id).fetch_one(&pool).await;
        let (max_mem, max_storage, max_cpu) = match limits { Ok(r) => (r.max_memory_mb, r.max_storage_gb, r.max_cpu_millicores), Err(_) => (0, 0, 0) };
        let namespace = format!("hermes-ws-{}", ws_id);
        let _ = K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage, max_cpu).await;

        let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
        let timestamp = Utc::now().timestamp();
        let full_image_tag = format!("{}/fn-{}:{}", registry_url, instance_id, timestamp);
        let mut kaniko_destination = full_image_tag.clone();
        if registry_url.contains("localhost") || registry_url.contains("127.0.0.1") {
            kaniko_destination = format!("registry.kube-system.svc.cluster.local:80/fn-{}:{}", instance_id, timestamp);
        }

        let configmap_name = format!("fn-build-context-{}", instance_id);
        let builder_pod_name = format!("fn-builder-{}", instance_id);
        let registry_secret_name = format!("hermes-registry-creds-fn-{}", instance_id);

        // Combined code of all routes for dependency extraction.
        let combined_code: String = routes.iter().map(|r| r.code.clone()).collect::<Vec<_>>().join("\n");

        let mut cm_data = std::collections::HashMap::new();
        if runtime.starts_with("nodejs") {
            let esm = runtime == "nodejs-esm";
            let dockerfile = "FROM node:20-alpine\nWORKDIR /app\nCOPY . ./\nRUN npm install --production\nEXPOSE 8080\nCMD [\"node\", \"index.js\"]".to_string();
            let mut parsed_deps = extract_dependencies(&combined_code, &runtime);
            parsed_deps.insert("express".to_string(), "^4.19.2".to_string());
            let package_json = serde_json::to_string_pretty(&json!({
                "name": "serverless-instance",
                "version": "1.0.0",
                "main": "index.js",
                "type": if esm { "module" } else { "commonjs" },
                "dependencies": parsed_deps
            })).unwrap_or_else(|_| "{}".to_string());

            cm_data.insert("Dockerfile".to_string(), dockerfile);
            cm_data.insert("package.json".to_string(), package_json);
            cm_data.insert("index.js".to_string(), generate_node_index(&routes, esm));
            for (i, r) in routes.iter().enumerate() {
                cm_data.insert(format!("route_{}.js", i), r.code.clone());
            }
        } else if runtime.starts_with("python") {
            let dockerfile = "FROM python:3.11-slim\nWORKDIR /app\nCOPY . ./\nRUN pip install --no-cache-dir -r requirements.txt\nEXPOSE 8080\nENV PORT=8080\nCMD [\"python\", \"index.py\"]".to_string();
            let parsed_deps = extract_dependencies(&combined_code, &runtime);
            let mut reqs = vec!["flask>=3.0.0".to_string()];
            for (dep, _) in parsed_deps { reqs.push(dep); }
            cm_data.insert("Dockerfile".to_string(), dockerfile);
            cm_data.insert("requirements.txt".to_string(), reqs.join("\n"));
            cm_data.insert("index.py".to_string(), generate_python_index(&routes));
            for (i, r) in routes.iter().enumerate() {
                cm_data.insert(format!("route_{}.py", i), r.code.clone());
            }
        }

        let mut copy_cmd = "cp".to_string();
        for key in cm_data.keys() { copy_cmd.push_str(&format!(" /configmap/{}", key)); }
        copy_cmd.push_str(" /workspace/");

        let configmaps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
        let cm_obj: ConfigMap = match serde_json::from_value(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": { "name": configmap_name, "namespace": namespace },
            "data": cm_data
        })) {
            Ok(o) => o,
            Err(e) => { let _ = save_build_error(&pool, instance_id, build_id, &format!("Eroare serializare ConfigMap: {}", e)).await; return; }
        };
        let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
        if let Err(e) = configmaps.create(&PostParams::default(), &cm_obj).await {
            let _ = save_build_error(&pool, instance_id, build_id, &format!("Eroare creare ConfigMap: {}", e)).await; return;
        }

        // Registry creds (optional)
        let registry_user = std::env::var("HERMES_REGISTRY_USER").ok();
        let registry_password = std::env::var("HERMES_REGISTRY_PASSWORD").ok();
        let mut has_registry_creds = false;
        let secrets_api: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        if let (Some(user), Some(pass)) = (registry_user, registry_password) {
            let auth = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{}:{}", user, pass));
            let docker_config = json!({ "auths": { registry_url.clone(): { "auth": auth }, "registry.kube-system.svc.cluster.local:80": { "auth": auth } } });
            let secret_manifest = json!({
                "apiVersion": "v1", "kind": "Secret",
                "metadata": { "name": registry_secret_name, "namespace": namespace },
                "type": "kubernetes.io/dockerconfigjson",
                "data": { ".dockerconfigjson": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, docker_config.to_string()) }
            });
            if let Ok(sec_obj) = serde_json::from_value(secret_manifest) {
                let _ = secrets_api.delete(&registry_secret_name, &DeleteParams::default()).await;
                if secrets_api.create(&PostParams::default(), &sec_obj).await.is_ok() { has_registry_creds = true; }
            }
        }

        let mut builder_pod_manifest = json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": { "name": builder_pod_name, "namespace": namespace, "labels": { "app": "hermes-fn-builder", "instance-id": instance_id.to_string() } },
            "spec": {
                "restartPolicy": "Never",
                "initContainers": [{
                    "name": "context-copier", "image": "alpine/git:latest",
                    "command": ["/bin/sh", "-c", &copy_cmd],
                    "volumeMounts": [{ "name": "configmap-volume", "mountPath": "/configmap" }, { "name": "context-volume", "mountPath": "/workspace" }]
                }],
                "containers": [{
                    "name": "kaniko", "image": "gcr.io/kaniko-project/executor:v1.14.0",
                    "args": ["--context=dir:///workspace", "--dockerfile=/workspace/Dockerfile", format!("--destination={}", kaniko_destination), "--skip-tls-verify", "--insecure"],
                    "volumeMounts": [{ "name": "context-volume", "mountPath": "/workspace" }],
                    "resources": { "requests": { "cpu": "100m", "memory": "256Mi" }, "limits": { "cpu": "1000m", "memory": "1024Mi" } }
                }],
                "volumes": [{ "name": "configmap-volume", "configMap": { "name": configmap_name } }, { "name": "context-volume", "emptyDir": {} }]
            }
        });
        if has_registry_creds {
            if let Some(spec) = builder_pod_manifest.get_mut("spec") {
                if let Some(c) = spec.get_mut("containers").and_then(|c| c.get_mut(0)).and_then(|c| c.get_mut("volumeMounts")).and_then(|m| m.as_array_mut()) {
                    c.push(json!({ "name": "registry-creds", "mountPath": "/kaniko/.docker" }));
                }
                if let Some(v) = spec.get_mut("volumes").and_then(|v| v.as_array_mut()) {
                    v.push(json!({ "name": "registry-creds", "secret": { "secretName": registry_secret_name } }));
                }
            }
        }

        let builder_pod: Pod = match serde_json::from_value(builder_pod_manifest) {
            Ok(p) => p,
            Err(e) => { let _ = save_build_error(&pool, instance_id, build_id, &format!("Eroare manifest builder: {}", e)).await; return; }
        };
        let pods_api: Api<Pod> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = pods_api.delete(&builder_pod_name, &DeleteParams::default()).await;
        if let Err(e) = pods_api.create(&PostParams::default(), &builder_pod).await {
            let _ = save_build_error(&pool, instance_id, build_id, &format!("Eroare lansare pod builder: {}", e)).await; return;
        }

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut success = false;
        for _ in 0..150 {
            interval.tick().await;
            if let Ok(pod) = pods_api.get(&builder_pod_name).await {
                if let Some(phase) = pod.status.and_then(|s| s.phase) {
                    if phase == "Succeeded" { success = true; break; }
                    if phase == "Failed" { break; }
                }
            } else { break; }
        }

        let mut build_logs = String::new();
        let lp = kube::api::LogParams { container: Some("kaniko".to_string()), ..Default::default() };
        match pods_api.logs(&builder_pod_name, &lp).await {
            Ok(logs) => build_logs.push_str(&logs),
            Err(e) => build_logs.push_str(&format!("Nu s-au putut prelua logurile builder-ului: {}\n", e)),
        }

        let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
        let _ = pods_api.delete(&builder_pod_name, &DeleteParams::default()).await;
        if has_registry_creds { let _ = secrets_api.delete(&registry_secret_name, &DeleteParams::default()).await; }

        let duration = start_time.elapsed().as_secs();
        let total_log = format!(
            "=========================================\n STAGE 1: COMPILING SERVERLESS INSTANCE ({} routes) [Duration: {}s]\n=========================================\n{}\n\n=========================================\n BUILD RESULT: {}\n=========================================",
            routes.len(), duration, build_logs, if success { "SUCCESS" } else { "FAILED" }
        );

        if !success {
            let _ = sqlx::query!("UPDATE serverless_instances SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2", total_log, instance_id).execute(&pool).await;
            let _ = sqlx::query!("UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3", total_log, duration as i32, build_id).execute(&pool).await;
            broadcast_instance(&pool, ws_id, instance_id).await;
            return;
        }

        // Stage 2: Knative deploy
        let k8s_svc_name = format!("fn-{}", slugify(&instance_name));
        let mut deployment_image = full_image_tag.clone();
        if registry_url.contains("192.168.") || registry_url.contains("127.0.0.1") || registry_url.contains("localhost") {
            deployment_image = deployment_image.replace(&registry_url, "localhost:5000");
        }
        let envs = resolve_instance_env_map(&pool, instance_id, project_id, inherit_project_envs).await;
        let deploy_res = K8sManager::deploy_knative_service(
            &k8s_client, &namespace, &k8s_svc_name, &deployment_image, envs, 0, 5, 10, Some(memory_limit_mb), None
        ).await;
        if let Err(e) = deploy_res {
            let final_log = format!("{}\n\n=========================================\n STAGE 2: DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!("UPDATE serverless_instances SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2", final_log, instance_id).execute(&pool).await;
            let _ = sqlx::query!("UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3", final_log, start_time.elapsed().as_secs() as i32, build_id).execute(&pool).await;
            broadcast_instance(&pool, ws_id, instance_id).await;
            return;
        }

        // Stage 3: Nginx proxy + LoadBalancer
        if let Err(e) = deploy_proxy_resources(&k8s_client, &namespace, &k8s_svc_name, external_port).await {
            let final_log = format!("{}\n\n=========================================\n STAGE 3: PROXY DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!("UPDATE serverless_instances SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2", final_log, instance_id).execute(&pool).await;
            let _ = sqlx::query!("UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3", final_log, start_time.elapsed().as_secs() as i32, build_id).execute(&pool).await;
            broadcast_instance(&pool, ws_id, instance_id).await;
            return;
        }

        if let Some(ref domain) = assigned_domain {
            let _ = K8sManager::deploy_ingress(&k8s_client, &namespace, &k8s_svc_name, domain, &format!("{}-proxy-svc", k8s_svc_name), 80).await;
        }

        let final_log = format!(
            "{}\n\n=========================================\n STAGE 2: DEPLOY SUCCESS\n=========================================\n- Knative Service: {} (Memory: {}Mi) -> OK\n- Nginx Routing Proxy: http://localhost:{} -> OK\n- Route Ingress: {} -> OK\n\nINSTANCE IS ONLINE AND SERVING {} ROUTE(S)!",
            total_log, k8s_svc_name, memory_limit_mb, external_port, assigned_domain.as_deref().unwrap_or("N/A"), routes.len()
        );
        let _ = sqlx::query!("UPDATE serverless_instances SET status = 'active', build_logs = $1, current_image_tag = $2, updated_at = now() WHERE id = $3", final_log, full_image_tag, instance_id).execute(&pool).await;
        let _ = sqlx::query!("UPDATE serverless_builds SET status = 'success', logs = $1, image_tag = $2, duration_sec = $3, updated_at = now() WHERE id = $4", final_log, full_image_tag, start_time.elapsed().as_secs() as i32, build_id).execute(&pool).await;
        broadcast_instance(&pool, ws_id, instance_id).await;
    });

    Ok(Json(serde_json::json!({ "buildId": build_id })))
}

async fn save_build_error(pool: &sqlx::PgPool, instance_id: Uuid, build_id: Uuid, error_msg: &str) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE serverless_instances SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2", error_msg, instance_id).execute(pool).await?;
    let _ = sqlx::query!("UPDATE serverless_builds SET status = 'failed', logs = $1, updated_at = now() WHERE id = $2", error_msg, build_id).execute(pool).await;
    if let Ok(inst) = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1").bind(instance_id).fetch_one(pool).await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: inst.workspace_id, instance: inst }
        );
    }
    Ok(())
}

// ============================ ENV (instance-level) ============================

/// Effective env for an instance's Knative service: inherit-all project vars (if
/// opted in) + selectively-linked pool vars + the instance's own vars (win).
async fn resolve_instance_env_map(
    pool: &sqlx::PgPool,
    instance_id: Uuid,
    project_id: Uuid,
    inherit: bool,
) -> Vec<(String, String)> {
    let mut env_map = std::collections::HashMap::new();

    if inherit {
        if let Ok(rows) = sqlx::query!("SELECT key, encrypted_value, nonce FROM project_env_variables WHERE project_id = $1", project_id).fetch_all(pool).await {
            for r in rows {
                if let Ok(v) = crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                    env_map.insert(r.key.to_uppercase(), v);
                }
            }
        }
    }

    if let Ok(rows) = sqlx::query!(
        "SELECT pev.key, pev.encrypted_value, pev.nonce
         FROM serverless_env_links sel
         JOIN project_env_variables pev ON pev.id = sel.project_env_id
         WHERE sel.instance_id = $1",
        instance_id
    ).fetch_all(pool).await {
        for r in rows {
            if let Ok(v) = crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                env_map.insert(r.key.to_uppercase(), v);
            }
        }
    }

    if let Ok(rows) = sqlx::query!("SELECT key, encrypted_value, nonce FROM serverless_env_variables WHERE instance_id = $1", instance_id).fetch_all(pool).await {
        for r in rows {
            if let Ok(v) = crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                env_map.insert(r.key.to_uppercase(), v);
            }
        }
    }

    let mut envs: Vec<(String, String)> = env_map.into_iter().collect();
    envs.sort_by(|a, b| a.0.cmp(&b.0));
    envs
}

/// POST /serverless/:id/reload-env — re-apply env on the running Knative service
/// without a rebuild (reuses current_image_tag; stamps a reload annotation).
pub async fn reload_instance_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<InstanceResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1")
        .bind(instance_id).fetch_one(&state.pool).await?;
    let image = inst.current_image_tag.clone().ok_or_else(|| AppError::Validation("Lansează instanța cel puțin o dată înainte de a reîncărca variabilele.".to_string()))?;

    let envs = resolve_instance_env_map(&state.pool, inst.id, inst.project_id, inst.inherit_project_envs).await;

    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&inst.name));

    let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
    let mut deployment_image = image.clone();
    if registry_url.contains("192.168.") || registry_url.contains("127.0.0.1") || registry_url.contains("localhost") {
        deployment_image = deployment_image.replace(&registry_url, "localhost:5000");
    }

    K8sManager::deploy_knative_service(
        &k8s_client, &namespace, &k8s_svc_name, &deployment_image, envs, 0, 5, 10,
        Some(inst.memory_limit_mb), Some(Utc::now().to_rfc3339())
    ).await?;

    Ok(Json(load_instance_response(&state.pool, inst).await))
}

fn to_instance_env_response(e: ServerlessEnvVariable) -> InstanceEnvResponse {
    let value = if !e.is_secret {
        crate::utils::crypto::decrypt_env_value(&e.encrypted_value, &e.nonce).ok()
    } else { None };
    InstanceEnvResponse { id: e.id, instance_id: e.instance_id, key: e.key, value, is_secret: e.is_secret }
}

pub async fn list_instance_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<InstanceEnvResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    let rows = sqlx::query_as::<_, ServerlessEnvVariable>("SELECT * FROM serverless_env_variables WHERE instance_id = $1 ORDER BY key ASC")
        .bind(instance_id).fetch_all(&state.pool).await?;
    Ok(Json(rows.into_iter().map(to_instance_env_response).collect()))
}

pub async fn set_instance_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<SetInstanceEnvRequest>,
) -> Result<(StatusCode, Json<InstanceEnvResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let clean_key = clean_env_key(&payload.key)?;
    let is_secret = payload.is_secret.unwrap_or(true);
    let (encrypted_value, nonce) = crate::utils::crypto::encrypt_env_value(&payload.value)?;
    let record_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO serverless_env_variables (id, workspace_id, instance_id, key, encrypted_value, nonce, is_secret)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (instance_id, key) DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, updated_at = now()",
        record_id, ws_id, instance_id, clean_key, encrypted_value, nonce, is_secret
    )
    .execute(&state.pool)
    .await?;

    Ok((StatusCode::OK, Json(InstanceEnvResponse {
        id: record_id, instance_id, key: clean_key,
        value: if is_secret { None } else { Some(payload.value) }, is_secret,
    })))
}

pub async fn delete_instance_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id, env_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    let affected = sqlx::query!("DELETE FROM serverless_env_variables WHERE id = $1 AND instance_id = $2", env_id, instance_id)
        .execute(&state.pool).await?.rows_affected();
    if affected == 0 { return Err(AppError::NotFound("Environment variable not found.".to_string())); }
    Ok(StatusCode::NO_CONTENT)
}

// ============================ PROJECT-POOL LINKS ============================

pub async fn list_instance_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ProjectEnvResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let rows = sqlx::query!(
        "SELECT pev.id, pev.project_id, pev.key, pev.encrypted_value, pev.nonce, pev.is_secret, pev.source,
                (sel.instance_id IS NOT NULL) AS \"linked!\"
         FROM project_env_variables pev
         LEFT JOIN serverless_env_links sel ON sel.project_env_id = pev.id AND sel.instance_id = $1
         WHERE pev.project_id = $2
         ORDER BY pev.key ASC",
        instance_id, project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let list = rows.into_iter().map(|r| {
        let value = if !r.is_secret { crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce).ok() } else { None };
        ProjectEnvResponse { id: r.id, project_id: r.project_id, key: r.key, value, is_secret: r.is_secret, source: r.source, linked: Some(r.linked) }
    }).collect();
    Ok(Json(list))
}

pub async fn link_instance_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<LinkProjectEnvRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    let belongs = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE id = $1 AND project_id = $2)",
        payload.project_env_id, project_id
    ).fetch_one(&state.pool).await?.unwrap_or(false);
    if !belongs { return Err(AppError::Permission("Project env var is not in this project.".to_string())); }
    sqlx::query!(
        "INSERT INTO serverless_env_links (instance_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        instance_id, payload.project_env_id
    ).execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unlink_instance_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id, project_env_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    sqlx::query!("DELETE FROM serverless_env_links WHERE instance_id = $1 AND project_env_id = $2", instance_id, project_env_id)
        .execute(&state.pool).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ============================ BUILDS + LOGS ============================

pub async fn list_instance_builds(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ServerlessBuildResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;
    let builds = sqlx::query_as::<_, ServerlessBuild>("SELECT * FROM serverless_builds WHERE instance_id = $1 ORDER BY created_at DESC LIMIT 50")
        .bind(instance_id).fetch_all(&state.pool).await?;
    let items = builds.into_iter().map(|b| ServerlessBuildResponse {
        id: b.id, status: b.status, image_tag: b.image_tag, duration_sec: b.duration_sec, created_at: b.created_at
    }).collect();
    Ok(Json(items))
}

pub async fn stream_build_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id, build_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let build = sqlx::query_as::<_, ServerlessBuild>("SELECT * FROM serverless_builds WHERE id = $1 AND instance_id = $2")
        .bind(build_id).bind(instance_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Build not found.".to_string()))?;

    let pool = state.pool.clone();
    let sse_stream = async_stream::stream! {
        if build.status != "building" {
            for line in build.logs.lines() { yield Ok(Event::default().data(line.to_string())); }
            return;
        }
        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => { yield Ok(Event::default().data(format!("[System] Conexiune Kubernetes eșuată: {}", e))); return; }
        };
        let namespace = format!("hermes-ws-{}", ws_id);
        let builder_pod_name = format!("fn-builder-{}", instance_id);
        let pods_api: kube::Api<Pod> = kube::Api::namespaced(k8s_client, &namespace);
        yield Ok(Event::default().data("=========================================\n COMPILARE INSTANȚĂ (KANIKO) — LIVE\n=========================================".to_string()));

        let mut pod_ready = false;
        for _ in 0..15 {
            if pods_api.get(&builder_pod_name).await.map(|p| p.status.is_some()).unwrap_or(false) { pod_ready = true; break; }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        if pod_ready {
            let log_params = kube::api::LogParams { container: Some("kaniko".to_string()), follow: true, ..Default::default() };
            if let Ok(log_stream) = pods_api.log_stream(&builder_pod_name, &log_params).await {
                use futures_util::io::AsyncBufReadExt;
                let mut lines = log_stream.lines();
                while let Some(line_res) = lines.next().await {
                    match line_res { Ok(line) => yield Ok(Event::default().data(line)), Err(_) => break }
                }
            }
        }
        let mut last_len = 0usize;
        for _ in 0..150 {
            if let Ok(row) = sqlx::query!("SELECT status, logs FROM serverless_builds WHERE id = $1", build_id).fetch_one(&pool).await {
                if row.logs.len() > last_len {
                    if let Some(appended) = row.logs.get(last_len..) {
                        for line in appended.lines() { yield Ok(Event::default().data(line.to_string())); }
                    }
                    last_len = row.logs.len();
                }
                if row.status != "building" { yield Ok(Event::default().data(format!("\n[System] Build {}.", row.status))); break; }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    };
    Ok(Sse::new(sse_stream))
}

pub async fn stream_instance_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    let inst = sqlx::query_as::<_, ServerlessInstance>("SELECT * FROM serverless_instances WHERE id = $1 AND workspace_id = $2")
        .bind(id).bind(ws_id).fetch_optional(&state.pool).await?
        .ok_or_else(|| AppError::NotFound("Instanță serverless negăsită.".to_string()))?;

    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&inst.name));

    let sse_stream = async_stream::stream! {
        let pods_api: Api<Pod> = Api::namespaced(k8s_client.clone(), &namespace);
        let lp = ListParams::default().labels(&format!("serving.knative.dev/service={}", k8s_svc_name));
        loop {
            let pod_list = match pods_api.list(&lp).await {
                Ok(list) => list,
                Err(e) => { yield Ok(Event::default().data(format!("[Console Error] Eșec listare pod-uri: {}", e))); tokio::time::sleep(std::time::Duration::from_secs(3)).await; continue; }
            };
            let pod = match pod_list.items.first() {
                Some(p) => p,
                None => { yield Ok(Event::default().data("[Console] Instanța este inactivă (scalată la zero) sau se redeploiază. Se așteaptă apelare...".to_string())); tokio::time::sleep(std::time::Duration::from_secs(3)).await; continue; }
            };
            let pod_name = match &pod.metadata.name { Some(name) => name.clone(), None => { tokio::time::sleep(std::time::Duration::from_secs(2)).await; continue; } };
            let phase = pod.status.as_ref().and_then(|s| s.phase.clone()).unwrap_or_else(|| "Unknown".to_string());
            if phase == "Pending" || phase == "Unknown" {
                yield Ok(Event::default().data(format!("[Console] Instanța se inițializează (Status: {})...", phase)));
                tokio::time::sleep(std::time::Duration::from_secs(3)).await; continue;
            }
            let log_params = kube::api::LogParams { follow: true, tail_lines: Some(100), container: Some("user-container".to_string()), ..Default::default() };
            match pods_api.log_stream(&pod_name, &log_params).await {
                Ok(log_stream) => {
                    yield Ok(Event::default().data("[Console] Conectat cu succes la fluxul de logs:".to_string()));
                    use futures_util::io::AsyncBufReadExt;
                    let mut lines = log_stream.lines();
                    while let Some(line_res) = lines.next().await {
                        match line_res { Ok(line) => yield Ok(Event::default().data(line)), Err(e) => { yield Ok(Event::default().data(format!("[Console Warning] Eroare rețea logs: {}", e))); break; } }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(e) => { yield Ok(Event::default().data(format!("[Console] Conectare la logs eșuată (se reîncearcă): {}", e))); tokio::time::sleep(std::time::Duration::from_secs(3)).await; }
            }
        }
    };
    Ok(Sse::new(sse_stream))
}

// ============================ METRICS ============================

#[derive(serde::Deserialize)]
pub struct InstanceMetricsQuery {
    pub metric: String,
    pub range: Option<String>,
}

/// GET /projects/:pid/serverless/:id/metrics — historical CPU/memory for the
/// instance's Knative pods (cAdvisor via Prometheus). Honest: empty + simulated
/// when Prometheus is unreachable, never fabricated.
pub async fn get_instance_metrics(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, instance_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<InstanceMetricsQuery>,
) -> Result<Json<crate::dtos::metrics_dto::MetricsHistoryResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_instance(&state.pool, project_id, instance_id, ws_id).await?;

    let name = sqlx::query_scalar!("SELECT name FROM serverless_instances WHERE id = $1", instance_id)
        .fetch_one(&state.pool).await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    // Knative pods are named fn-<slug>-<rev>-deployment-...; the cAdvisor queries
    // match on pod=~"<container>-.*", so the Knative service name is the right key.
    let container_name = format!("fn-{}", slugify(&name));
    let range = query.range.unwrap_or_else(|| "1h".to_string());

    let (timestamps, values, simulated) = crate::utils::prometheus::get_historical_metrics(
        &namespace, &container_name, &query.metric, &range, "serverless"
    ).await?;

    Ok(Json(crate::dtos::metrics_dto::MetricsHistoryResponse { timestamps, values, simulated }))
}

// ============================ HELPERS ============================

async fn get_random_available_port(pool: &sqlx::PgPool) -> Result<i32, AppError> {
    for _ in 0..100 {
        let port: i32 = (rand::random::<u32>() % 10000 + 20000) as i32;
        let in_apps = sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM app_instances WHERE external_port = $1)", port).fetch_one(pool).await?.unwrap_or(false);
        let in_dbs = sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM databases WHERE external_port = $1 AND is_external = true)", port).fetch_one(pool).await?.unwrap_or(false);
        let in_fns = sqlx::query_scalar!("SELECT EXISTS(SELECT 1 FROM serverless_instances WHERE external_port = $1)", port).fetch_one(pool).await?.unwrap_or(false);
        if !in_apps && !in_dbs && !in_fns { return Ok(port); }
    }
    Err(AppError::Fatal(anyhow::anyhow!("Could not allocate a free external port after 100 attempts.")))
}

async fn deploy_proxy_resources(
    client: &Client,
    namespace: &str,
    ksvc_name: &str,
    external_port: i32,
) -> Result<(), AppError> {
    let configmap_name = format!("{}-proxy-config", ksvc_name);
    let deployment_name = format!("{}-proxy", ksvc_name);
    let service_name = format!("{}-external", ksvc_name);
    let proxy_svc_name = format!("{}-proxy-svc", ksvc_name);

    // Server-side apply = create-or-update, idempotent on redeploy. The old
    // delete-then-create raced on the LoadBalancer Service: delete leaves it in
    // Terminating (LB-cleanup finalizer) and the immediate create hit 409 AlreadyExists.
    let apply_pp = PatchParams::apply("hermes-serverless").force();

    // Forward everything to the Knative service; method/route enforcement now lives
    // inside the instance's wrapper (per-route), so no method check here.
    let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    // Cluster DNS IP differs by distro (k3s: 10.43.0.10, vanilla k8s: 10.96.0.10).
    // Read it from the kube-dns Service so nginx can actually resolve the Knative
    // service name — a hardcoded 10.96.0.10 silently 502s on k3s.
    let dns_ip = {
        let svcs: Api<k8s_openapi::api::core::v1::Service> = Api::namespaced(client.clone(), "kube-system");
        svcs.get("kube-dns").await.ok()
            .and_then(|s| s.spec)
            .and_then(|sp| sp.cluster_ip)
            .filter(|ip| !ip.is_empty() && ip != "None")
            .unwrap_or_else(|| "10.43.0.10".to_string())
    };
    let nginx_conf = format!(
        r#"events {{ worker_connections 1024; }}
http {{
    client_max_body_size 0;
    resolver {} valid=5s;
    server {{
        listen 8080;
        location / {{
            set $backend_url "http://{}.{}.svc.cluster.local";
            proxy_pass $backend_url;
            proxy_set_header Host {}.{}.svc.cluster.local;
            proxy_http_version 1.1;
            proxy_set_header Connection "";
            proxy_read_timeout 600s;
            proxy_connect_timeout 600s;
        }}
    }}
}}"#,
        dns_ip, ksvc_name, namespace, ksvc_name, namespace
    );

    let cm_obj: ConfigMap = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": { "name": configmap_name, "namespace": namespace },
        "data": { "nginx.conf": nginx_conf }
    })).map_err(|e| AppError::Fatal(anyhow::anyhow!("ConfigMap JSON serialization failed: {}", e)))?;
    configmaps.patch(&configmap_name, &apply_pp, &Patch::Apply(&cm_obj)).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply proxy configmap: {}", e)))?;

    let deployments: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(client.clone(), namespace);
    let depl_obj: k8s_openapi::api::apps::v1::Deployment = serde_json::from_value(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": { "name": deployment_name, "namespace": namespace, "labels": { "app": format!("{}-proxy", ksvc_name) } },
        "spec": {
            "replicas": 1,
            "selector": { "matchLabels": { "app": format!("{}-proxy", ksvc_name) } },
            "template": {
                "metadata": { "labels": { "app": format!("{}-proxy", ksvc_name) } },
                "spec": {
                    "containers": [{
                        "name": "nginx", "image": "nginx:alpine",
                        "ports": [{ "containerPort": 8080 }],
                        "volumeMounts": [{ "name": "config-volume", "mountPath": "/etc/nginx/nginx.conf", "subPath": "nginx.conf" }],
                        "resources": { "requests": { "cpu": "25m", "memory": "32Mi" }, "limits": { "cpu": "100m", "memory": "64Mi" } }
                    }],
                    "volumes": [{ "name": "config-volume", "configMap": { "name": configmap_name } }]
                }
            }
        }
    })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Deployment JSON serialization failed: {}", e)))?;
    deployments.patch(&deployment_name, &apply_pp, &Patch::Apply(&depl_obj)).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply proxy deployment: {}", e)))?;

    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    let svc_obj: Service = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": service_name, "namespace": namespace, "labels": { "app": format!("{}-proxy", ksvc_name) } },
        "spec": { "type": "LoadBalancer", "ports": [{ "name": "http", "port": external_port, "targetPort": 8080, "protocol": "TCP" }], "selector": { "app": format!("{}-proxy", ksvc_name) } }
    })).map_err(|e| AppError::Fatal(anyhow::anyhow!("Service JSON serialization failed: {}", e)))?;
    services.patch(&service_name, &apply_pp, &Patch::Apply(&svc_obj)).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply proxy service: {}", e)))?;

    let svc_cluster_obj: Service = serde_json::from_value(json!({
        "apiVersion": "v1", "kind": "Service",
        "metadata": { "name": proxy_svc_name, "namespace": namespace, "labels": { "app": format!("{}-proxy", ksvc_name) } },
        "spec": { "type": "ClusterIP", "ports": [{ "name": "http", "port": 80, "targetPort": 8080, "protocol": "TCP" }], "selector": { "app": format!("{}-proxy", ksvc_name) } }
    })).map_err(|e| AppError::Fatal(anyhow::anyhow!("ClusterIP Service JSON serialization failed: {}", e)))?;
    services.patch(&proxy_svc_name, &apply_pp, &Patch::Apply(&svc_cluster_obj)).await
        .map_err(|e| AppError::Infrastructure(format!("Failed to apply proxy cluster ip service: {}", e)))?;

    Ok(())
}

fn sanitize_npm_package_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.starts_with('/') { return None; }
    let mut parts = name.split('/');
    let first = parts.next()?;
    if first.starts_with('@') {
        let second = parts.next()?;
        Some(format!("{}/{}", first, second))
    } else {
        Some(first.to_string())
    }
}

fn extract_js_dependencies(code: &str) -> std::collections::HashSet<String> {
    let mut deps = std::collections::HashSet::new();
    let builtins: std::collections::HashSet<&str> = [
        "assert", "async_hooks", "buffer", "child_process", "cluster", "console", "constants",
        "crypto", "dgram", "dns", "domain", "events", "fs", "fs/promises", "http", "http2",
        "https", "inspector", "module", "net", "os", "path", "perf_hooks", "process",
        "punycode", "querystring", "readline", "repl", "stream", "string_decoder",
        "timers", "tls", "trace_events", "tty", "url", "util", "v8", "vm", "wasi",
        "worker_threads", "zlib"
    ].iter().cloned().collect();

    let chars: Vec<char> = code.chars().collect();
    let len = chars.len();
    let mut i = 0;
    while i < len {
        if i + 7 < len && chars[i..i+7] == ['r','e','q','u','i','r','e'] {
            let mut p = i + 7;
            while p < len && chars[p].is_whitespace() { p += 1; }
            if p < len && chars[p] == '(' {
                p += 1;
                while p < len && chars[p].is_whitespace() { p += 1; }
                if p < len && (chars[p] == '\'' || chars[p] == '"' || chars[p] == '`') {
                    let quote = chars[p];
                    p += 1;
                    let start = p;
                    while p < len && chars[p] != quote { p += 1; }
                    if p < len {
                        let name: String = chars[start..p].iter().collect();
                        if let Some(pkg) = sanitize_npm_package_name(&name) {
                            if !builtins.contains(pkg.as_str()) { deps.insert(pkg); }
                        }
                    }
                }
            }
            i += 7;
            continue;
        }
        if i + 6 < len && chars[i..i+6] == ['i','m','p','o','r','t'] {
            let mut p = i + 6;
            let mut found_from = false;
            let mut is_dynamic = false;
            let mut temp = p;
            while temp < len && chars[temp].is_whitespace() { temp += 1; }
            if temp < len && chars[temp] == '(' { is_dynamic = true; p = temp + 1; }
            if is_dynamic {
                while p < len && chars[p].is_whitespace() { p += 1; }
                if p < len && (chars[p] == '\'' || chars[p] == '"' || chars[p] == '`') {
                    let quote = chars[p];
                    p += 1;
                    let start = p;
                    while p < len && chars[p] != quote { p += 1; }
                    if p < len {
                        let name: String = chars[start..p].iter().collect();
                        if let Some(pkg) = sanitize_npm_package_name(&name) {
                            if !builtins.contains(pkg.as_str()) { deps.insert(pkg); }
                        }
                    }
                }
            } else {
                while p < len {
                    if chars[p] == ';' || chars[p] == '\n' { break; }
                    if p + 4 < len && chars[p..p+4] == ['f','r','o','m'] && chars[p-1].is_whitespace() && chars[p+4].is_whitespace() {
                        found_from = true; p += 4; break;
                    }
                    p += 1;
                }
                if found_from {
                    while p < len && chars[p].is_whitespace() { p += 1; }
                    if p < len && (chars[p] == '\'' || chars[p] == '"' || chars[p] == '`') {
                        let quote = chars[p];
                        p += 1;
                        let start = p;
                        while p < len && chars[p] != quote { p += 1; }
                        if p < len {
                            let name: String = chars[start..p].iter().collect();
                            if let Some(pkg) = sanitize_npm_package_name(&name) {
                                if !builtins.contains(pkg.as_str()) { deps.insert(pkg); }
                            }
                        }
                    }
                }
            }
            i += 6;
            continue;
        }
        i += 1;
    }
    deps
}

fn extract_python_dependencies(code: &str) -> std::collections::HashSet<String> {
    let mut deps = std::collections::HashSet::new();
    let builtins: std::collections::HashSet<&str> = [
        "os", "sys", "time", "json", "math", "re", "datetime", "random", "hashlib",
        "urllib", "http", "socket", "subprocess", "threading", "multiprocessing",
        "collections", "itertools", "functools", "pathlib", "flask", "logging"
    ].iter().cloned().collect();

    for line in code.lines() {
        let line = line.trim();
        if line.starts_with("import ") {
            let imports = line[7..].split(',');
            for imp in imports {
                let part = imp.trim().split(" as ").next().unwrap_or("").trim();
                let base = part.split('.').next().unwrap_or("").trim();
                if !base.is_empty() && !builtins.contains(base) { deps.insert(base.to_string()); }
            }
        } else if line.starts_with("from ") {
            let part = line[5..].split(" import ").next().unwrap_or("").trim();
            let base = part.split('.').next().unwrap_or("").trim();
            if !base.is_empty() && !builtins.contains(base) { deps.insert(base.to_string()); }
        }
    }
    deps
}

fn extract_dependencies(code: &str, runtime: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if runtime.starts_with("nodejs") {
        for dep in extract_js_dependencies(code) { map.insert(dep, "*".to_string()); }
    } else if runtime.starts_with("python") {
        for dep in extract_python_dependencies(code) { map.insert(dep, "*".to_string()); }
    }
    map
}
