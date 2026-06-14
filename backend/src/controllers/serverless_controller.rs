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
use kube::{api::{ListParams, PostParams, DeleteParams, PatchParams, Patch}, Api, Client};
use k8s_openapi::api::core::v1::{ConfigMap, Pod, Service};
use std::time::Instant;

use crate::app_state::AppState;
use crate::models::serverless_model::{ServerlessFunction, ServerlessBuild};
use crate::dtos::serverless_dto::{CreateFunctionRequest, UpdateFunctionRequest, FunctionResponse, ServerlessBuildResponse};
use crate::dtos::env_variable_dto::{ProjectEnvResponse, LinkProjectEnvRequest};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{error::AppError, k8s::K8sManager};
use crate::utils::pagination::{PaginationParams, Paginated};

pub fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace(' ', "-")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-')
        .collect()
}

pub async fn list_functions(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<FunctionResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM serverless_functions WHERE workspace_id = $1 AND project_id = $2",
        ws_id, project_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(0);

    let functions = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE workspace_id = $1 AND project_id = $2 ORDER BY name ASC LIMIT $3 OFFSET $4"
    )
    .bind(ws_id)
    .bind(project_id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let items = functions.into_iter().map(to_response).collect();
    Ok(Json(Paginated::new(items, total, page, page_size)))
}

pub async fn create_function(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(project_id): Path<Uuid>,
    Json(payload): Json<CreateFunctionRequest>,
) -> Result<(StatusCode, Json<FunctionResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let slug = slugify(&payload.name);
    if slug.is_empty() {
        return Err(AppError::Validation("Function name is invalid.".to_string()));
    }

    let runtime = payload.runtime.clone().unwrap_or_else(|| "nodejs-cjs".to_string());

    let default_code = payload.code.unwrap_or_else(|| {
        if runtime == "nodejs-esm" {
            "export default async function(req, res) {\n    res.status(200).json({\n        success: true,\n        message: \"Hello from ES Modules Serverless function!\"\n    });\n};".to_string()
        } else if runtime == "python" {
            "def handler(request):\n    return {\n        \"success\": True,\n        \"message\": \"Hello from Python Serverless function!\"\n    }".to_string()
        } else {
            "module.exports = async (req, res) => {\n    res.status(200).json({\n        success: true,\n        message: \"Hello from Serverless function!\"\n    });\n};".to_string()
        }
    });

    let memory = payload.memory_limit_mb.unwrap_or(0);

    // Check workspace memory limits
    crate::utils::limits::check_workspace_memory_limit(
        &state.pool,
        ws_id,
        memory as i64,
        None
    ).await?;

    let route_path = if payload.route_path.starts_with('/') {
        payload.route_path.clone()
    } else {
        format!("/{}", payload.route_path)
    };

    let id = Uuid::new_v4();

    sqlx::query!(
        "INSERT INTO serverless_functions (id, workspace_id, project_id, name, code, method, route_path, memory_limit_mb, status, runtime, inherit_project_envs)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'draft', $9, false)",
        id, ws_id, project_id, payload.name.trim(), default_code, payload.method.to_uppercase(), route_path, memory, runtime
    )
    .execute(&state.pool)
    .await?;

    let function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    let res_fn = to_response(function.clone());

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated {
            workspace_id: ws_id,
            function,
        }
    );

    Ok((StatusCode::CREATED, Json(res_fn)))
}

pub async fn get_function(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<FunctionResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Serverless function not found.".to_string()))?;

    Ok(Json(to_response(function)))
}

pub async fn update_function(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<UpdateFunctionRequest>,
) -> Result<Json<FunctionResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let mut function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Serverless function not found.".to_string()))?;

    let name = payload.name.unwrap_or(function.name);
    let code = payload.code.unwrap_or(function.code);
    let method = payload.method.unwrap_or(function.method).to_uppercase();
    let memory = payload.memory_limit_mb.unwrap_or(function.memory_limit_mb);

    // Check workspace memory limits
    if payload.memory_limit_mb.is_some() {
        crate::utils::limits::check_workspace_memory_limit(
            &state.pool,
            ws_id,
            memory as i64,
            Some(id)
        ).await?;
    }
    let env_variables = payload.env_variables.unwrap_or(function.env_variables);
    
    let route_path = if let Some(p) = payload.route_path {
        if p.starts_with('/') { p } else { format!("/{}", p) }
    } else {
        function.route_path
    };

    let assigned_domain = match payload.assigned_domain.clone() {
        Some(domain_opt) => domain_opt,
        None => function.assigned_domain.clone(),
    };

    if let Some(ref domain_opt) = payload.assigned_domain {
        if let Some(fqdn) = domain_opt {
            let fqdn_clean = fqdn.to_lowercase().trim().to_string();
            if !fqdn_clean.is_empty() {
                // Check if the domain is registered in the workspace
                let domain_exists = sqlx::query_scalar!(
                    "SELECT EXISTS(SELECT 1 FROM domains WHERE workspace_id = $1 AND fqdn = $2)",
                    ws_id, fqdn_clean
                )
                .fetch_one(&state.pool)
                .await?
                .unwrap_or(false);

                if !domain_exists {
                    return Err(AppError::Validation("Domeniul selectat nu este înregistrat în acest workspace.".to_string()));
                }

                // 1. Clear any other domains that were previously targeting this serverless function
                let _ = sqlx::query!(
                    "UPDATE domains SET target_type = 'custom', target_id = NULL, nginx_target_host = NULL 
                     WHERE workspace_id = $1 AND target_type = 'serverless' AND target_id = $2 AND fqdn != $3",
                    ws_id, id, fqdn_clean
                )
                .execute(&state.pool)
                .await;

                // 2. Associate this domain with this function
                let svc_name = format!("fn-{}-proxy-svc", slugify(&name));
                let _ = sqlx::query!(
                    "UPDATE domains SET target_type = 'serverless', target_id = $1, routing_type = 'reverse_proxy', nginx_target_host = $2, nginx_root_path = NULL, nginx_config_content = NULL 
                     WHERE workspace_id = $3 AND fqdn = $4",
                    id, svc_name, ws_id, fqdn_clean
                )
                .execute(&state.pool)
                .await;
            }
        } else {
            // Disassociate all domains targeting this serverless function
            let _ = sqlx::query!(
                "UPDATE domains SET target_type = 'custom', target_id = NULL, nginx_target_host = NULL 
                 WHERE workspace_id = $1 AND target_type = 'serverless' AND target_id = $2",
                ws_id, id
            )
            .execute(&state.pool)
            .await;
        }
    }

    // Ensure the domains table's target host matches the function name (in case it changed)
    let svc_name = format!("fn-{}-proxy-svc", slugify(&name));
    let _ = sqlx::query!(
        "UPDATE domains SET nginx_target_host = $1 
         WHERE workspace_id = $2 AND target_type = 'serverless' AND target_id = $3",
        svc_name, ws_id, id
    )
    .execute(&state.pool)
    .await;

    let runtime = payload.runtime.unwrap_or(function.runtime);
    let inherit_project_envs = payload.inherit_project_envs.unwrap_or(function.inherit_project_envs);

    sqlx::query!(
        "UPDATE serverless_functions
         SET name = $1, code = $2, method = $3, route_path = $4, memory_limit_mb = $5, env_variables = $6, assigned_domain = $7, runtime = $8, inherit_project_envs = $9, status = 'draft', updated_at = now()
         WHERE id = $10",
        name, code, method, route_path, memory, env_variables, assigned_domain, runtime, inherit_project_envs, id
    )
    .execute(&state.pool)
    .await?;

    let updated_fn = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    // Publish the function's public URL into the project pool so apps can opt in.
    // Only meaningful once a domain is assigned; the URL value tracks the function.
    // `publish_project_env` keys off (source, source_id), so it refreshes the value
    // in place and preserves any key the user renamed in the Environments UI — the
    // suggested key below is only used on the very first publish.
    let suggested_key = format!("{}_FUNCTION_URL", crate::utils::app_env::sanitize_key_fragment(&updated_fn.name, "FUNCTION"));
    if let Some(domain) = updated_fn.assigned_domain.as_ref().filter(|d| !d.trim().is_empty()) {
        let url = format!("https://{}{}", domain.trim(), updated_fn.route_path);
        let _ = crate::utils::app_env::publish_project_env(
            &state.pool, ws_id, updated_fn.project_id, &suggested_key, &url, false, "serverless", id
        ).await;
        // The value may have changed for already-linked instances; reload them.
        if let Ok(insts) = sqlx::query_scalar!(
            "SELECT ael.app_instance_id FROM app_env_links ael
             JOIN project_env_variables pev ON pev.id = ael.project_env_id
             WHERE pev.source = 'serverless' AND pev.source_id = $1",
            id
        )
        .fetch_all(&state.pool)
        .await
        {
            for inst in insts {
                crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
            }
        }
    }

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated {
            workspace_id: ws_id,
            function: updated_fn.clone(),
        }
    );

    Ok(Json(to_response(updated_fn)))
}

pub async fn delete_function(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Serverless function not found.".to_string()))?;

    // Remove the function's published project-pool var and reload linked apps.
    let linked = crate::utils::app_env::unpublish_project_env(&state.pool, "serverless", id).await;
    for inst in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
    }

    // Delete in DB
    sqlx::query!("DELETE FROM serverless_functions WHERE id = $1", id)
        .execute(&state.pool)
        .await?;

    // Clean up Knative & Ingress from Kubernetes asynchronously
    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&function.name));

    tokio::spawn(async move {
        let _ = K8sManager::delete_knative_service(&k8s_client, &namespace, &k8s_svc_name).await;
        let _ = K8sManager::delete_ingress(&k8s_client, &namespace, &k8s_svc_name).await;

        // Clean up proxy resources
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = configmaps.delete(&format!("{}-proxy-config", k8s_svc_name), &DeleteParams::default()).await;

        let deployments: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = deployments.delete(&format!("{}-proxy", k8s_svc_name), &DeleteParams::default()).await;

        let services: Api<Service> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = services.delete(&format!("{}-external", k8s_svc_name), &DeleteParams::default()).await;
        let _ = services.delete(&format!("{}-proxy-svc", k8s_svc_name), &DeleteParams::default()).await;
    });

    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionDeleted {
            workspace_id: ws_id,
            function_id: id,
        }
    );

    Ok(StatusCode::NO_CONTENT)
}

pub async fn deploy_function(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Serverless function not found.".to_string()))?;

    // Assign a port if not already assigned
    let external_port = match function.external_port {
        Some(port) => port,
        None => {
            let port = get_random_available_port(&state.pool).await?;
            sqlx::query!(
                "UPDATE serverless_functions SET external_port = $1 WHERE id = $2",
                port, id
            )
            .execute(&state.pool)
            .await?;
            port
        }
    };

    // Update status to building
    sqlx::query!("UPDATE serverless_functions SET status = 'building', updated_at = now() WHERE id = $1", id)
        .execute(&state.pool)
        .await?;

    // Announce 'building' so the global build indicator picks it up immediately.
    if let Ok(building_fn) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1")
        .bind(id)
        .fetch_one(&state.pool)
        .await
    {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated {
                workspace_id: ws_id,
                function: building_fn,
            }
        );
    }

    // Record a build in the history; its Kaniko logs can be streamed live and replayed later.
    let build_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO serverless_builds (id, function_id, workspace_id, status) VALUES ($1, $2, $3, 'building')",
        build_id, function.id, ws_id
    )
    .execute(&state.pool)
    .await?;

    let pool = state.pool.clone();
    let function_id = function.id;
    let function_name = function.name.clone();
    let function_code = function.code.clone();
    let function_method = function.method.clone();
    let memory_limit_mb = function.memory_limit_mb;
    let env_variables = function.env_variables.clone();
    let assigned_domain = function.assigned_domain.clone();
    let external_port = external_port;
    let runtime = function.runtime.clone();
    let inherit_project_envs = function.inherit_project_envs;
    let project_id = function.project_id;

    tokio::spawn(async move {
        let start_time = Instant::now();
        let k8s_client = match K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, build_id, &format!("Eșec conexiune Kubernetes: {}", e)).await;
                return;
            }
        };

        let limits = sqlx::query!("SELECT max_memory_mb, max_storage_gb FROM workspaces WHERE id = $1", ws_id)
            .fetch_one(&pool)
            .await;
        let (max_mem, max_storage) = match limits {
            Ok(r) => (r.max_memory_mb, r.max_storage_gb),
            Err(_) => (2048, 10),
        };
        let namespace = format!("hermes-ws-{}", ws_id);
        let _ = K8sManager::create_namespace(&k8s_client, &namespace, max_mem, max_storage).await;

        let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
        let timestamp = Utc::now().timestamp();
        let full_image_tag = format!("{}/fn-{}:{}", registry_url, function_id, timestamp);

        // For Kaniko running inside the cluster, localhost/127.0.0.1 registry must be accessed via the internal registry service
        let mut kaniko_destination = full_image_tag.clone();
        if registry_url.contains("localhost") || registry_url.contains("127.0.0.1") {
            kaniko_destination = format!("registry.kube-system.svc.cluster.local:80/fn-{}:{}", function_id, timestamp);
        }

        let configmap_name = format!("fn-build-context-{}", function_id);
        let builder_pod_name = format!("fn-builder-{}", function_id);
        // Per-function secret name so concurrent serverless builds don't delete each other's registry credentials.
        let registry_secret_name = format!("hermes-registry-creds-fn-{}", function_id);

        // Files to package inside ConfigMap
        let mut cm_data = std::collections::HashMap::new();
        let mut dockerfile = String::new();

        if runtime.starts_with("nodejs") {
            dockerfile = "FROM node:20-alpine\nWORKDIR /app\nCOPY package.json index.js function.js ./\nRUN npm install --production\nEXPOSE 8080\nCMD [\"node\", \"index.js\"]".to_string();
            
            // Extract dependencies from code
            let mut parsed_deps = extract_dependencies(&function_code, &runtime);
            parsed_deps.insert("express".to_string(), "^4.19.2".to_string());
            let package_json_obj = json!({
                "name": "serverless-function",
                "version": "1.0.0",
                "main": "index.js",
                "type": if runtime == "nodejs-esm" { "module" } else { "commonjs" },
                "dependencies": parsed_deps
            });
            let package_json = serde_json::to_string_pretty(&package_json_obj).unwrap_or_else(|_| "{}".to_string());
            
            let index_js = if runtime == "nodejs-esm" {
                r#"import express from 'express';
import handler from './function.js';

const app = express();
const port = process.env.PORT || 8080;

app.use(express.json());
app.use(express.urlencoded({ extended: true }));

const ALLOWED_METHOD = process.env.ALLOWED_METHOD || 'GET';

app.all('*', async (req, res) => {
    if (ALLOWED_METHOD !== 'ANY' && req.method !== ALLOWED_METHOD) {
        return res.status(405).json({ error: `Method ${req.method} Not Allowed. Expected ${ALLOWED_METHOD}.` });
    }
    try {
        await handler(req, res);
    } catch (err) {
        console.error('Handler error:', err);
        if (!res.headersSent) {
            res.status(500).json({ error: err.message || 'Internal Server Error' });
        }
    }
});

app.listen(port, () => {
    console.log(`Serverless function listening on port ${port}`);
});"#
            } else {
                r#"const express = require('express');
const app = express();
const port = process.env.PORT || 8080;

app.use(express.json());
app.use(express.urlencoded({ extended: true }));

const handler = require('./function.js');
const ALLOWED_METHOD = process.env.ALLOWED_METHOD || 'GET';

app.all('*', async (req, res) => {
    if (ALLOWED_METHOD !== 'ANY' && req.method !== ALLOWED_METHOD) {
        return res.status(405).json({ error: `Method ${req.method} Not Allowed. Expected ${ALLOWED_METHOD}.` });
    }
    try {
        await handler(req, res);
    } catch (err) {
        console.error('Handler error:', err);
        if (!res.headersSent) {
            res.status(500).json({ error: err.message || 'Internal Server Error' });
        }
    }
});

app.listen(port, () => {
    console.log(`Serverless function listening on port ${port}`);
});"#
            };

            cm_data.insert("Dockerfile".to_string(), dockerfile);
            cm_data.insert("package.json".to_string(), package_json);
            cm_data.insert("index.js".to_string(), index_js.to_string());
            cm_data.insert("function.js".to_string(), function_code);
        } else if runtime.starts_with("python") {
            dockerfile = "FROM python:3.11-slim\nWORKDIR /app\nCOPY requirements.txt index.py function.py ./\nRUN pip install --no-cache-dir -r requirements.txt\nEXPOSE 8080\nENV PORT=8080\nCMD [\"python\", \"index.py\"]".to_string();
            
            // Extract requirements
            let parsed_deps = extract_dependencies(&function_code, &runtime);
            let mut reqs = vec!["flask>=3.0.0".to_string()];
            for (dep, _) in parsed_deps {
                reqs.push(dep);
            }
            let requirements_txt = reqs.join("\n");
            
            let index_py = r#"import os
import sys
from flask import Flask, request, jsonify

app = Flask(__name__)
sys.path.append(os.path.dirname(os.path.abspath(__file__)))

try:
    import function
    handler = function.handler
except AttributeError:
    try:
        import function
        if hasattr(function, 'main'):
            handler = function.main
        else:
            handler = getattr(function, [k for k in dir(function) if not k.startswith('_') and callable(getattr(function, k))][0])
    except Exception as e:
        raise RuntimeError(f"Could not find entrypoint in function.py: {e}")

ALLOWED_METHOD = os.environ.get("ALLOWED_METHOD", "GET")

@app.route('/', defaults={'path': ''}, methods=['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'OPTIONS'])
@app.route('/<path:path>', methods=['GET', 'POST', 'PUT', 'DELETE', 'PATCH', 'OPTIONS'])
def catch_all(path):
    if ALLOWED_METHOD != 'ANY' and request.method != ALLOWED_METHOD:
        return jsonify({"error": f"Method {request.method} Not Allowed. Expected {ALLOWED_METHOD}."}), 405
    try:
        res = handler(request)
        if isinstance(res, (dict, list)):
            return jsonify(res)
        return res
    except Exception as err:
        return jsonify({"error": str(err)}), 500

if __name__ == '__main__':
    port = int(os.environ.get('PORT', 8080))
    app.run(host='0.0.0.0', port=port)"#;

            cm_data.insert("Dockerfile".to_string(), dockerfile);
            cm_data.insert("requirements.txt".to_string(), requirements_txt);
            cm_data.insert("index.py".to_string(), index_py.to_string());
            cm_data.insert("function.py".to_string(), function_code);
        }

        // Dynamically build the init container copy command
        let mut copy_cmd = "cp".to_string();
        for key in cm_data.keys() {
            copy_cmd.push_str(&format!(" /configmap/{}", key));
        }
        copy_cmd.push_str(" /workspace/");

        // Create ConfigMap containing files
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
        let cm_manifest = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": configmap_name,
                "namespace": namespace
            },
            "data": cm_data
        });

        let cm_obj: ConfigMap = match serde_json::from_value(cm_manifest) {
            Ok(o) => o,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, build_id, &format!("Eroare serializare ConfigMap: {}", e)).await;
                return;
            }
        };

        let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
        if let Err(e) = configmaps.create(&PostParams::default(), &cm_obj).await {
            let _ = save_build_error(&pool, function_id, build_id, &format!("Eroare creare ConfigMap în Kubernetes: {}", e)).await;
            return;
        }

        // Set up registry credentials if configured
        let registry_user = std::env::var("HERMES_REGISTRY_USER").ok();
        let registry_password = std::env::var("HERMES_REGISTRY_PASSWORD").ok();
        let mut has_registry_creds = false;
        
        let secrets_api: Api<k8s_openapi::api::core::v1::Secret> = Api::namespaced(k8s_client.clone(), &namespace);
        if let (Some(user), Some(pass)) = (registry_user, registry_password) {
            let auth = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, format!("{}:{}", user, pass));
            let docker_config = json!({
                "auths": {
                    registry_url.clone(): {
                        "auth": auth
                    },
                    "registry.kube-system.svc.cluster.local:80": {
                        "auth": auth
                    }
                }
            });

            let secret_manifest = json!({
                "apiVersion": "v1",
                "kind": "Secret",
                "metadata": {
                    "name": registry_secret_name,
                    "namespace": namespace
                },
                "type": "kubernetes.io/dockerconfigjson",
                "data": {
                    ".dockerconfigjson": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, docker_config.to_string())
                }
            });

            if let Ok(sec_obj) = serde_json::from_value(secret_manifest) {
                let _ = secrets_api.delete(&registry_secret_name, &DeleteParams::default()).await;
                if secrets_api.create(&PostParams::default(), &sec_obj).await.is_ok() {
                    has_registry_creds = true;
                }
            }
        }

        // Create Kaniko Pod Manifest
        let mut builder_pod_manifest = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": builder_pod_name,
                "namespace": namespace,
                "labels": {
                    "app": "hermes-fn-builder",
                    "function-id": function_id.to_string()
                }
            },
            "spec": {
                "restartPolicy": "Never",
                "initContainers": [{
                    "name": "context-copier",
                    "image": "alpine/git:latest",
                    "command": ["/bin/sh", "-c", &copy_cmd],
                    "volumeMounts": [{
                        "name": "configmap-volume",
                        "mountPath": "/configmap"
                    }, {
                        "name": "context-volume",
                        "mountPath": "/workspace"
                    }]
                }],
                "containers": [{
                    "name": "kaniko",
                    "image": "gcr.io/kaniko-project/executor:v1.14.0",
                    "args": [
                        "--context=dir:///workspace",
                        "--dockerfile=/workspace/Dockerfile",
                        format!("--destination={}", kaniko_destination),
                        "--skip-tls-verify",
                        "--insecure"
                    ],
                    "volumeMounts": [{
                        "name": "context-volume",
                        "mountPath": "/workspace"
                    }],
                    "resources": {
                        "requests": {
                            "cpu": "100m",
                            "memory": "256Mi"
                        },
                        "limits": {
                            "cpu": "1000m",
                            "memory": "1024Mi"
                        }
                    }
                }],
                "volumes": [{
                    "name": "configmap-volume",
                    "configMap": {
                        "name": configmap_name
                    }
                }, {
                    "name": "context-volume",
                    "emptyDir": {}
                }]
            }
        });

        if has_registry_creds {
            if let Some(spec) = builder_pod_manifest.get_mut("spec") {
                if let Some(containers) = spec.get_mut("containers") {
                    if let Some(kaniko_container) = containers.get_mut(0) {
                        if let Some(mounts) = kaniko_container.get_mut("volumeMounts") {
                            if let Some(mounts_arr) = mounts.as_array_mut() {
                                mounts_arr.push(json!({
                                    "name": "registry-creds",
                                    "mountPath": "/kaniko/.docker"
                                }));
                            }
                        }
                    }
                }
                if let Some(volumes) = spec.get_mut("volumes") {
                    if let Some(volumes_arr) = volumes.as_array_mut() {
                        volumes_arr.push(json!({
                            "name": "registry-creds",
                            "secret": {
                                "secretName": registry_secret_name
                            }
                        }));
                    }
                }
            }
        }

        let builder_pod: Pod = match serde_json::from_value(builder_pod_manifest) {
            Ok(p) => p,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, build_id, &format!("Eroare manifest builder: {}", e)).await;
                return;
            }
        };

        let pods_api: Api<Pod> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = pods_api.delete(&builder_pod_name, &DeleteParams::default()).await;

        if let Err(e) = pods_api.create(&PostParams::default(), &builder_pod).await {
            let _ = save_build_error(&pool, function_id, build_id, &format!("Eroare lansare pod builder: {}", e)).await;
            return;
        }

        // Wait for Builder Pod
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut success = false;
        for _ in 0..150 { // max 5 minutes
            interval.tick().await;
            if let Ok(pod) = pods_api.get(&builder_pod_name).await {
                if let Some(status) = pod.status {
                    if let Some(phase) = status.phase {
                        if phase == "Succeeded" {
                            success = true;
                            break;
                        }
                        if phase == "Failed" {
                            break;
                        }
                    }
                }
            } else {
                break;
            }
        }

        // Get logs
        let mut build_logs = String::new();
        let lp = kube::api::LogParams {
            container: Some("kaniko".to_string()),
            ..Default::default()
        };
        match pods_api.logs(&builder_pod_name, &lp).await {
            Ok(logs) => build_logs.push_str(&logs),
            Err(e) => build_logs.push_str(&format!("Nu s-au putut prelua logurile builder-ului: {}\n", e)),
        }

        // Clean up ConfigMap & Pod & Secret
        let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
        let _ = pods_api.delete(&builder_pod_name, &DeleteParams::default()).await;
        if has_registry_creds {
            let _ = secrets_api.delete(&registry_secret_name, &DeleteParams::default()).await;
        }

        let duration = start_time.elapsed().as_secs();
        let total_log = format!(
            "=========================================\n STAGE 1: COMPILING SERVERLESS FUNCTION [Duration: {}s]\n=========================================\n{}\n\n=========================================\n BUILD RESULT: {}\n=========================================",
            duration,
            build_logs,
            if success { "SUCCESS" } else { "FAILED" }
        );

        if !success {
            let _ = sqlx::query!(
                "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
                total_log, function_id
            )
            .execute(&pool)
            .await;
            let _ = sqlx::query!(
                "UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3",
                total_log, duration as i32, build_id
            )
            .execute(&pool)
            .await;

            if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(function_id).fetch_one(&pool).await {
                crate::utils::event_broadcaster::broadcast_event(
                    crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, function: func }
                );
            }
            return;
        }

        // Stage 2: Deploy to Knative
        let k8s_svc_name = format!("fn-{}", slugify(&function_name));
        
        let mut deployment_image = full_image_tag.clone();
        if registry_url.contains("192.168.") || registry_url.contains("127.0.0.1") || registry_url.contains("localhost") {
            deployment_image = deployment_image.replace(&registry_url, "localhost:5000");
        }

        // Project pool (inherit-all + selective links) + manual override + ALLOWED_METHOD.
        let envs = resolve_function_env_map(&pool, function_id, project_id, inherit_project_envs, &env_variables, &function_method).await;

        let deploy_res = K8sManager::deploy_knative_service(
            &k8s_client,
            &namespace,
            &k8s_svc_name,
            &deployment_image,
            envs,
            0, // minScale
            5, // maxScale
            10, // targetConcurrency
            Some(memory_limit_mb),
            None, // normal deploy: new image tag already forces a fresh revision
        ).await;

        if let Err(e) = deploy_res {
            let final_log = format!("{}\n\n=========================================\n STAGE 2: DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!(
                "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
                final_log, function_id
            )
            .execute(&pool)
            .await;
            let _ = sqlx::query!(
                "UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3",
                final_log, start_time.elapsed().as_secs() as i32, build_id
            )
            .execute(&pool)
            .await;

            if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(function_id).fetch_one(&pool).await {
                crate::utils::event_broadcaster::broadcast_event(
                    crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, function: func }
                );
            }
            return;
        }

        // Stage 3: Deploy Nginx Routing Proxy and LoadBalancer Service
        let proxy_res = deploy_proxy_resources(&k8s_client, &namespace, &k8s_svc_name, external_port, &function_method).await;
        if let Err(e) = proxy_res {
            let final_log = format!("{}\n\n=========================================\n STAGE 3: PROXY DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!(
                "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
                final_log, function_id
            )
            .execute(&pool)
            .await;
            let _ = sqlx::query!(
                "UPDATE serverless_builds SET status = 'failed', logs = $1, duration_sec = $2, updated_at = now() WHERE id = $3",
                final_log, start_time.elapsed().as_secs() as i32, build_id
            )
            .execute(&pool)
            .await;

            if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(function_id).fetch_one(&pool).await {
                crate::utils::event_broadcaster::broadcast_event(
                    crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, function: func }
                );
            }
            return;
        }

        // Deploy ingress if assigned domain is set
        if let Some(ref domain) = assigned_domain {
            let _ = K8sManager::deploy_ingress(
                &k8s_client,
                &namespace,
                &k8s_svc_name,
                domain,
                &format!("{}-proxy-svc", k8s_svc_name),
                80
            ).await;
        }

        let final_log = format!(
            "{}\n\n=========================================\n STAGE 2: DEPLOY SUCCESS\n=========================================\n- Knative Service: {} (Memory: {}Mi) -> OK\n- Nginx Routing Proxy: http://localhost:{} -> OK\n- Route Ingress: {} -> OK\n\nFUNCTION IS ONLINE AND READY TO SERVE REQUESTS!",
            total_log,
            k8s_svc_name,
            memory_limit_mb,
            external_port,
            assigned_domain.as_deref().unwrap_or("N/A")
        );

        let _ = sqlx::query!(
            "UPDATE serverless_functions SET status = 'active', build_logs = $1, current_image_tag = $2, updated_at = now() WHERE id = $3",
            final_log, full_image_tag, function_id
        )
        .execute(&pool)
        .await;
        let _ = sqlx::query!(
            "UPDATE serverless_builds SET status = 'success', logs = $1, image_tag = $2, duration_sec = $3, updated_at = now() WHERE id = $4",
            final_log, full_image_tag, start_time.elapsed().as_secs() as i32, build_id
        )
        .execute(&pool)
        .await;

        if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(function_id).fetch_one(&pool).await {
            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, function: func }
            );
        }
    });

    Ok(Json(serde_json::json!({ "buildId": build_id })))
}

async fn save_build_error(pool: &sqlx::PgPool, id: Uuid, build_id: Uuid, error_msg: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
        error_msg, id
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query!(
        "UPDATE serverless_builds SET status = 'failed', logs = $1, updated_at = now() WHERE id = $2",
        error_msg, build_id
    )
    .execute(pool)
    .await;

    if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(id).fetch_one(pool).await {
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: func.workspace_id, function: func }
        );
    }
    Ok(())
}

pub async fn stream_function_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((_project_id, id)): Path<(Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let function = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1 AND workspace_id = $2"
    )
    .bind(id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Serverless function not found.".to_string()))?;

    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&function.name));

    let sse_stream = async_stream::stream! {
        let pods_api: Api<Pod> = Api::namespaced(k8s_client.clone(), &namespace);
        let lp = ListParams::default().labels(&format!("serving.knative.dev/service={}", k8s_svc_name));

        loop {
            let pod_list = match pods_api.list(&lp).await {
                Ok(list) => list,
                Err(e) => {
                    yield Ok(Event::default().data(format!("[Console Error] Eșec listare pod-uri: {}", e)));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    continue;
                }
            };

            let pod = match pod_list.items.first() {
                Some(p) => p,
                None => {
                    yield Ok(Event::default().data("[Console] Funcția este inactivă (scalată la zero) sau se redeploiază. Se așteaptă apelare...".to_string()));
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
                yield Ok(Event::default().data(format!("[Console] Funcția se inițializează (Status: {})...", phase)));
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                continue;
            }

            let log_params = kube::api::LogParams {
                follow: true,
                tail_lines: Some(100),
                container: Some("user-container".to_string()), // Knative serves user code in 'user-container'
                ..Default::default()
            };

            let log_stream_res = pods_api.log_stream(&pod_name, &log_params).await;
            match log_stream_res {
                Ok(log_stream) => {
                    yield Ok(Event::default().data("[Console] Conectat cu succes la fluxul de logs:".to_string()));
                    
                    use futures_util::io::AsyncBufReadExt;
                    let mut lines = log_stream.lines();
                    
                    while let Some(line_res) = lines.next().await {
                        match line_res {
                            Ok(line) => {
                                yield Ok(Event::default().data(line));
                            }
                            Err(e) => {
                                yield Ok(Event::default().data(format!("[Console Warning] Eroare rețea logs: {}", e)));
                                break;
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
                Err(e) => {
                    yield Ok(Event::default().data(format!("[Console] Conectare la logs eșuată (se reîncearcă): {}", e)));
                    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                }
            }
        }
    };

    Ok(Sse::new(sse_stream))
}

/// Build the effective env for a function's Knative service: inherit-all project
/// vars (if opted in) + selectively-linked pool vars + manual vars (win) + the
/// ALLOWED_METHOD system var. Shared by deploy and env-only reload.
async fn resolve_function_env_map(
    pool: &sqlx::PgPool,
    function_id: Uuid,
    project_id: Uuid,
    inherit: bool,
    env_variables: &serde_json::Value,
    method: &str,
) -> Vec<(String, String)> {
    let mut env_map = std::collections::HashMap::new();

    if inherit {
        if let Ok(rows) = sqlx::query!(
            "SELECT key, encrypted_value, nonce FROM project_env_variables WHERE project_id = $1",
            project_id
        )
        .fetch_all(pool)
        .await
        {
            for r in rows {
                if let Ok(v) = crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                    env_map.insert(r.key.to_uppercase(), v);
                }
            }
        }
    }

    // Selectively-linked project pool vars (parity with apps' app_env_links).
    if let Ok(rows) = sqlx::query!(
        "SELECT pev.key, pev.encrypted_value, pev.nonce
         FROM serverless_env_links sel
         JOIN project_env_variables pev ON pev.id = sel.project_env_id
         WHERE sel.function_id = $1",
        function_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                env_map.insert(r.key.to_uppercase(), v);
            }
        }
    }

    if let Some(arr) = env_variables.as_array() {
        for val in arr {
            if let (Some(k), Some(v)) = (val.get("key").and_then(|k| k.as_str()), val.get("value").and_then(|v| v.as_str())) {
                env_map.insert(k.to_uppercase(), v.to_string());
            }
        }
    }
    env_map.insert("ALLOWED_METHOD".to_string(), method.to_string());

    let mut envs: Vec<(String, String)> = env_map.into_iter().collect();
    envs.sort_by(|a, b| a.0.cmp(&b.0));
    envs
}

/// POST /projects/:project_id/functions/:id/reload-env — re-apply env on the
/// running Knative service WITHOUT a rebuild (reuses current_image_tag; stamps a
/// reload annotation to force a fresh revision that re-reads the env secret).
pub async fn reload_function_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<FunctionResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    let function = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1")
        .bind(function_id)
        .fetch_one(&state.pool)
        .await?;

    let image = function.current_image_tag.clone().ok_or_else(|| {
        AppError::Validation("Lansează funcția cel puțin o dată înainte de a reîncărca variabilele.".to_string())
    })?;

    let envs = resolve_function_env_map(
        &state.pool, function.id, function.project_id, function.inherit_project_envs, &function.env_variables, &function.method
    ).await;

    let k8s_client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", ws_id);
    let k8s_svc_name = format!("fn-{}", slugify(&function.name));

    // Mirror the deploy-time registry rewrite for in-cluster localhost registries.
    let registry_url = std::env::var("HERMES_REGISTRY_URL").unwrap_or_else(|_| "localhost:5000".to_string());
    let mut deployment_image = image.clone();
    if registry_url.contains("192.168.") || registry_url.contains("127.0.0.1") || registry_url.contains("localhost") {
        deployment_image = deployment_image.replace(&registry_url, "localhost:5000");
    }

    let reload_token = Utc::now().timestamp().to_string();
    K8sManager::deploy_knative_service(
        &k8s_client, &namespace, &k8s_svc_name, &deployment_image, envs,
        0, 5, 10, Some(function.memory_limit_mb), Some(reload_token),
    ).await?;

    let _ = sqlx::query!("UPDATE serverless_functions SET updated_at = now() WHERE id = $1", function_id)
        .execute(&state.pool)
        .await;

    Ok(Json(to_response(function)))
}

/// GET /projects/:pid/functions/:id/builds — recent build history for a function.
pub async fn list_function_builds(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ServerlessBuildResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    let builds = sqlx::query_as::<_, ServerlessBuild>(
        "SELECT * FROM serverless_builds WHERE function_id = $1 ORDER BY created_at DESC LIMIT 50"
    )
    .bind(function_id)
    .fetch_all(&state.pool)
    .await?;

    let items = builds.into_iter().map(|b| ServerlessBuildResponse {
        id: b.id,
        status: b.status,
        image_tag: b.image_tag,
        duration_sec: b.duration_sec,
        created_at: b.created_at,
    }).collect();
    Ok(Json(items))
}

/// GET /projects/:pid/functions/:id/builds/:build_id/logs/stream — live Kaniko build
/// logs (follow the fn-builder pod); replays stored logs for completed builds.
pub async fn stream_build_logs(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id, build_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    let build = sqlx::query_as::<_, ServerlessBuild>(
        "SELECT * FROM serverless_builds WHERE id = $1 AND function_id = $2"
    )
    .bind(build_id)
    .bind(function_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Build not found.".to_string()))?;

    let pool = state.pool.clone();

    let sse_stream = async_stream::stream! {
        // Completed build: replay stored logs and stop.
        if build.status != "building" {
            for line in build.logs.lines() {
                yield Ok(Event::default().data(line.to_string()));
            }
            return;
        }

        let k8s_client = match crate::utils::k8s::K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => { yield Ok(Event::default().data(format!("[System] Conexiune Kubernetes eșuată: {}", e))); return; }
        };
        let namespace = format!("hermes-ws-{}", ws_id);
        let builder_pod_name = format!("fn-builder-{}", function_id);
        let pods_api: kube::Api<Pod> = kube::Api::namespaced(k8s_client, &namespace);

        yield Ok(Event::default().data("=========================================\n COMPILARE FUNCȚIE (KANIKO) — LIVE\n=========================================".to_string()));

        // Wait for the builder pod to be scheduled (max ~30s).
        let mut pod_ready = false;
        for _ in 0..15 {
            if pods_api.get(&builder_pod_name).await.map(|p| p.status.is_some()).unwrap_or(false) {
                pod_ready = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        if pod_ready {
            let log_params = kube::api::LogParams { container: Some("kaniko".to_string()), follow: true, ..Default::default() };
            if let Ok(log_stream) = pods_api.log_stream(&builder_pod_name, &log_params).await {
                use futures_util::io::AsyncBufReadExt;
                let mut lines = log_stream.lines();
                while let Some(line_res) = lines.next().await {
                    match line_res {
                        Ok(line) => yield Ok(Event::default().data(line)),
                        Err(_) => break,
                    }
                }
            }
        }

        // After the build pod ends, poll the DB until the deploy records the final
        // status/logs (Knative + proxy stages run after Kaniko), then emit & stop.
        let mut last_len = 0usize;
        for _ in 0..150 {
            if let Ok(row) = sqlx::query!("SELECT status, logs FROM serverless_builds WHERE id = $1", build_id).fetch_one(&pool).await {
                if row.logs.len() > last_len {
                    if let Some(appended) = row.logs.get(last_len..) {
                        for line in appended.lines() {
                            yield Ok(Event::default().data(line.to_string()));
                        }
                    }
                    last_len = row.logs.len();
                }
                if row.status != "building" {
                    yield Ok(Event::default().data(format!("\n[System] Build {}.", row.status)));
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    };

    Ok(Sse::new(sse_stream))
}

/// Verify a function belongs to the given project within the caller's workspace.
async fn authorize_function(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    function_id: Uuid,
    ws_id: Uuid,
) -> Result<(), AppError> {
    let ok = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM serverless_functions WHERE id = $1 AND project_id = $2 AND workspace_id = $3)",
        function_id, project_id, ws_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    if !ok {
        return Err(AppError::NotFound("Serverless function not found in this project.".to_string()));
    }
    Ok(())
}

/// GET /projects/:project_id/functions/:id/project-env — the project pool with a
/// `linked` flag per var for this function (parity with instance project-env).
pub async fn list_function_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<ProjectEnvResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    let rows = sqlx::query!(
        "SELECT pev.id, pev.project_id, pev.key, pev.encrypted_value, pev.nonce, pev.is_secret, pev.source,
                (sel.function_id IS NOT NULL) AS \"linked!\"
         FROM project_env_variables pev
         LEFT JOIN serverless_env_links sel
           ON sel.project_env_id = pev.id AND sel.function_id = $1
         WHERE pev.project_id = $2
         ORDER BY pev.key ASC",
        function_id, project_id
    )
    .fetch_all(&state.pool)
    .await?;

    let list = rows
        .into_iter()
        .map(|r| {
            let value = if !r.is_secret {
                crate::utils::crypto::decrypt_env_value(&r.encrypted_value, &r.nonce).ok()
            } else {
                None
            };
            ProjectEnvResponse {
                id: r.id,
                project_id: r.project_id,
                key: r.key,
                value,
                is_secret: r.is_secret,
                source: r.source,
                linked: Some(r.linked),
            }
        })
        .collect();

    Ok(Json(list))
}

/// POST /projects/:project_id/functions/:id/env-links — link a pool var to a function.
pub async fn link_function_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id)): Path<(Uuid, Uuid)>,
    Json(payload): Json<LinkProjectEnvRequest>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    let belongs = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM project_env_variables WHERE id = $1 AND project_id = $2)",
        payload.project_env_id, project_id
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);
    if !belongs {
        return Err(AppError::Permission("Project env var is not in this function's project.".to_string()));
    }

    sqlx::query!(
        "INSERT INTO serverless_env_links (function_id, project_env_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        function_id, payload.project_env_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /projects/:project_id/functions/:id/env-links/:project_env_id — unlink.
pub async fn unlink_function_project_env(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path((project_id, function_id, project_env_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| AppError::Validation("No active workspace selected.".to_string()))?;
    authorize_function(&state.pool, project_id, function_id, ws_id).await?;

    sqlx::query!(
        "DELETE FROM serverless_env_links WHERE function_id = $1 AND project_env_id = $2",
        function_id, project_env_id
    )
    .execute(&state.pool)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

fn to_response(f: ServerlessFunction) -> FunctionResponse {
    FunctionResponse {
        id: f.id,
        workspace_id: f.workspace_id,
        project_id: f.project_id,
        name: f.name,
        code: f.code,
        method: f.method,
        route_path: f.route_path,
        memory_limit_mb: f.memory_limit_mb,
        env_variables: f.env_variables,
        status: f.status,
        assigned_domain: f.assigned_domain,
        build_logs: f.build_logs,
        external_port: f.external_port,
        runtime: f.runtime,
        inherit_project_envs: f.inherit_project_envs,
        created_at: f.created_at,
        updated_at: f.updated_at,
    }
}

async fn get_random_available_port(pool: &sqlx::PgPool) -> Result<i32, AppError> {
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

        let port_in_use_fns = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM serverless_functions WHERE external_port = $1)",
            port
        )
        .fetch_one(pool)
        .await?
        .unwrap_or(false);

        if !port_in_use_apps && !port_in_use_dbs && !port_in_use_fns {
            return Ok(port);
        }
    }
    Err(AppError::Fatal(anyhow::anyhow!("Could not allocate a free external port after 100 attempts.")))
}

async fn deploy_proxy_resources(
    client: &Client,
    namespace: &str,
    ksvc_name: &str,
    external_port: i32,
    allowed_method: &str,
) -> Result<(), AppError> {
    let configmap_name = format!("{}-proxy-config", ksvc_name);
    let deployment_name = format!("{}-proxy", ksvc_name);
    let service_name = format!("{}-external", ksvc_name);
    let proxy_svc_name = format!("{}-proxy-svc", ksvc_name);

    let method_check = if allowed_method != "ANY" {
        format!(
            "        if ($request_method !~ ^({}|OPTIONS)$) {{\n            return 405;\n        }}\n",
            allowed_method.to_uppercase()
        )
    } else {
        "".to_string()
    };

    // 1. ConfigMap for Nginx
    let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let nginx_conf = format!(
        r#"events {{ worker_connections 1024; }}
http {{
    client_max_body_size 0;
    resolver 10.96.0.10 valid=5s;
    server {{
        listen 8080;
{}        location / {{
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
        method_check, ksvc_name, namespace, ksvc_name, namespace
    );

    let cm_manifest = json!({
        "apiVersion": "v1",
        "kind": "ConfigMap",
        "metadata": {
            "name": configmap_name,
            "namespace": namespace
        },
        "data": {
            "nginx.conf": nginx_conf
        }
    });

    let cm_obj: ConfigMap = serde_json::from_value(cm_manifest)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("ConfigMap JSON serialization failed: {}", e)))?;
    let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
    configmaps.create(&PostParams::default(), &cm_obj)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to create proxy configmap: {}", e)))?;

    // 2. Deployment for Nginx
    let deployments: Api<k8s_openapi::api::apps::v1::Deployment> = Api::namespaced(client.clone(), namespace);
    let depl_manifest = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": deployment_name,
            "namespace": namespace,
            "labels": {
                "app": format!("{}-proxy", ksvc_name)
            }
        },
        "spec": {
            "replicas": 1,
            "selector": {
                "matchLabels": {
                    "app": format!("{}-proxy", ksvc_name)
                }
            },
            "template": {
                "metadata": {
                    "labels": {
                        "app": format!("{}-proxy", ksvc_name)
                    }
                },
                "spec": {
                    "containers": [{
                        "name": "nginx",
                        "image": "nginx:alpine",
                        "ports": [{
                            "containerPort": 8080
                        }],
                        "volumeMounts": [{
                            "name": "config-volume",
                            "mountPath": "/etc/nginx/nginx.conf",
                            "subPath": "nginx.conf"
                        }],
                        "resources": {
                            "requests": {
                                "cpu": "25m",
                                "memory": "32Mi"
                            },
                            "limits": {
                                "cpu": "100m",
                                "memory": "64Mi"
                            }
                        }
                    }],
                    "volumes": [{
                        "name": "config-volume",
                        "configMap": {
                            "name": configmap_name
                        }
                    }]
                }
            }
        }
    });

    let depl_obj: k8s_openapi::api::apps::v1::Deployment = serde_json::from_value(depl_manifest)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Deployment JSON serialization failed: {}", e)))?;
    let _ = deployments.delete(&deployment_name, &DeleteParams::default()).await;
    deployments.create(&PostParams::default(), &depl_obj)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to create proxy deployment: {}", e)))?;

    // 3. Service of type LoadBalancer (External port)
    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    let svc_manifest = json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {
            "name": service_name,
            "namespace": namespace,
            "labels": {
                "app": format!("{}-proxy", ksvc_name)
            }
        },
        "spec": {
            "type": "LoadBalancer",
            "ports": [{
                "name": "http",
                "port": external_port,
                "targetPort": 8080,
                "protocol": "TCP"
            }],
            "selector": {
                "app": format!("{}-proxy", ksvc_name)
            }
        }
    });

    let svc_obj: Service = serde_json::from_value(svc_manifest)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Service JSON serialization failed: {}", e)))?;
    let _ = services.delete(&service_name, &DeleteParams::default()).await;
    services.create(&PostParams::default(), &svc_obj)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to create proxy service: {}", e)))?;

    // 4. Service of type ClusterIP (Port 80 for Ingress routing)
    let svc_cluster_manifest = json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {
            "name": proxy_svc_name,
            "namespace": namespace,
            "labels": {
                "app": format!("{}-proxy", ksvc_name)
            }
        },
        "spec": {
            "type": "ClusterIP",
            "ports": [{
                "name": "http",
                "port": 80,
                "targetPort": 8080,
                "protocol": "TCP"
            }],
            "selector": {
                "app": format!("{}-proxy", ksvc_name)
            }
        }
    });

    let svc_cluster_obj: Service = serde_json::from_value(svc_cluster_manifest)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("ClusterIP Service JSON serialization failed: {}", e)))?;
    let _ = services.delete(&proxy_svc_name, &DeleteParams::default()).await;
    services.create(&PostParams::default(), &svc_cluster_obj)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to create proxy cluster ip service: {}", e)))?;

    Ok(())
}

fn sanitize_npm_package_name(name: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() || name.starts_with('.') || name.starts_with('/') {
        return None;
    }
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
                            if !builtins.contains(pkg.as_str()) {
                                deps.insert(pkg);
                            }
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
            if temp < len && chars[temp] == '(' {
                is_dynamic = true;
                p = temp + 1;
            }

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
                            if !builtins.contains(pkg.as_str()) {
                                deps.insert(pkg);
                            }
                        }
                    }
                }
            } else {
                while p < len {
                    if chars[p] == ';' || chars[p] == '\n' {
                        break;
                    }
                    if p + 4 < len && chars[p..p+4] == ['f','r','o','m'] && chars[p-1].is_whitespace() && chars[p+4].is_whitespace() {
                        found_from = true;
                        p += 4;
                        break;
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
                                if !builtins.contains(pkg.as_str()) {
                                    deps.insert(pkg);
                                }
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
                if !base.is_empty() && !builtins.contains(base) {
                    deps.insert(base.to_string());
                }
            }
        } else if line.starts_with("from ") {
            let part = line[5..].split(" import ").next().unwrap_or("").trim();
            let base = part.split('.').next().unwrap_or("").trim();
            if !base.is_empty() && !builtins.contains(base) {
                deps.insert(base.to_string());
            }
        }
    }
    deps
}

fn extract_dependencies(code: &str, runtime: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if runtime.starts_with("nodejs") {
        let extracted = extract_js_dependencies(code);
        for dep in extracted {
            map.insert(dep, "*".to_string());
        }
    } else if runtime.starts_with("python") {
        let extracted = extract_python_dependencies(code);
        for dep in extracted {
            map.insert(dep, "*".to_string());
        }
    }
    map
}
