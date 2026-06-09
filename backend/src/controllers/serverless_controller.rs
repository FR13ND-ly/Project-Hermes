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
use crate::models::serverless_model::ServerlessFunction;
use crate::dtos::serverless_dto::{CreateFunctionRequest, UpdateFunctionRequest, FunctionResponse};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{error::AppError, k8s::K8sManager};

fn slugify(name: &str) -> String {
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
) -> Result<Json<Vec<FunctionResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let functions = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE workspace_id = $1 AND project_id = $2 ORDER BY name ASC"
    )
    .bind(ws_id)
    .bind(project_id)
    .fetch_all(&state.pool)
    .await?;

    let resp = functions.into_iter().map(to_response).collect();
    Ok(Json(resp))
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

    let default_code = payload.code.unwrap_or_else(|| {
        "module.exports = async (req, res) => {\n    res.status(200).json({\n        success: true,\n        message: \"Hello from Serverless function!\"\n    });\n};".to_string()
    });

    let memory = payload.memory_limit_mb.unwrap_or(128);

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
        "INSERT INTO serverless_functions (id, workspace_id, project_id, name, code, method, route_path, memory_limit_mb, status)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'draft')",
        id, ws_id, project_id, payload.name.trim(), default_code, payload.method.to_uppercase(), route_path, memory
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

    let assigned_domain = match payload.assigned_domain {
        Some(domain_opt) => domain_opt,
        None => function.assigned_domain,
    };

    sqlx::query!(
        "UPDATE serverless_functions
         SET name = $1, code = $2, method = $3, route_path = $4, memory_limit_mb = $5, env_variables = $6, assigned_domain = $7, status = 'draft', updated_at = now()
         WHERE id = $8",
        name, code, method, route_path, memory, env_variables, assigned_domain, id
    )
    .execute(&state.pool)
    .await?;

    let updated_fn = sqlx::query_as::<_, ServerlessFunction>(
        "SELECT * FROM serverless_functions WHERE id = $1"
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

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

    let pool = state.pool.clone();
    let function_id = function.id;
    let function_name = function.name.clone();
    let function_code = function.code.clone();
    let function_method = function.method.clone();
    let memory_limit_mb = function.memory_limit_mb;
    let env_variables = function.env_variables.clone();
    let assigned_domain = function.assigned_domain.clone();
    let external_port = external_port;

    tokio::spawn(async move {
        let start_time = Instant::now();
        let k8s_client = match K8sManager::get_client().await {
            Ok(c) => c,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, &format!("Eșec conexiune Kubernetes: {}", e)).await;
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

        // Files to package inside ConfigMap
        let dockerfile = "FROM node:20-alpine\nWORKDIR /app\nCOPY package.json index.js function.js ./\nRUN npm install --production\nEXPOSE 8080\nCMD [\"node\", \"index.js\"]";
        let package_json = "{\n  \"name\": \"serverless-function\",\n  \"version\": \"1.0.0\",\n  \"main\": \"index.js\",\n  \"dependencies\": {\n    \"express\": \"^4.19.2\"\n  }\n}";
        let index_js = "const express = require('express');\nconst app = express();\nconst port = process.env.PORT || 8080;\n\napp.use(express.json());\napp.use(express.urlencoded({ extended: true }));\n\nconst handler = require('./function.js');\nconst ALLOWED_METHOD = process.env.ALLOWED_METHOD || 'GET';\n\napp.all('*', async (req, res) => {\n    if (ALLOWED_METHOD !== 'ANY' && req.method !== ALLOWED_METHOD) {\n        return res.status(405).json({ error: `Method ${req.method} Not Allowed. Expected ${ALLOWED_METHOD}.` });\n    }\n    try {\n        await handler(req, res);\n    } catch (err) {\n        console.error('Handler error:', err);\n        if (!res.headersSent) {\n            res.status(500).json({ error: err.message || 'Internal Server Error' });\n        }\n    }\n});\n\napp.listen(port, () => {\n    console.log(`Serverless function listening on port ${port}`);\n});";

        // Create ConfigMap containing files
        let configmaps: Api<ConfigMap> = Api::namespaced(k8s_client.clone(), &namespace);
        let cm_manifest = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": configmap_name,
                "namespace": namespace
            },
            "data": {
                "Dockerfile": dockerfile,
                "package.json": package_json,
                "index.js": index_js,
                "function.js": function_code
            }
        });

        let cm_obj: ConfigMap = match serde_json::from_value(cm_manifest) {
            Ok(o) => o,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, &format!("Eroare serializare ConfigMap: {}", e)).await;
                return;
            }
        };

        let _ = configmaps.delete(&configmap_name, &DeleteParams::default()).await;
        if let Err(e) = configmaps.create(&PostParams::default(), &cm_obj).await {
            let _ = save_build_error(&pool, function_id, &format!("Eroare creare ConfigMap în Kubernetes: {}", e)).await;
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
                    "name": "hermes-registry-credentials",
                    "namespace": namespace
                },
                "type": "kubernetes.io/dockerconfigjson",
                "data": {
                    ".dockerconfigjson": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, docker_config.to_string())
                }
            });

            if let Ok(sec_obj) = serde_json::from_value(secret_manifest) {
                let _ = secrets_api.delete("hermes-registry-credentials", &DeleteParams::default()).await;
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
                    "command": ["/bin/sh", "-c", "cp /configmap/Dockerfile /configmap/package.json /configmap/index.js /configmap/function.js /workspace/"],
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
                                "secretName": "hermes-registry-credentials"
                            }
                        }));
                    }
                }
            }
        }

        let builder_pod: Pod = match serde_json::from_value(builder_pod_manifest) {
            Ok(p) => p,
            Err(e) => {
                let _ = save_build_error(&pool, function_id, &format!("Eroare manifest builder: {}", e)).await;
                return;
            }
        };

        let pods_api: Api<Pod> = Api::namespaced(k8s_client.clone(), &namespace);
        let _ = pods_api.delete(&builder_pod_name, &DeleteParams::default()).await;

        if let Err(e) = pods_api.create(&PostParams::default(), &builder_pod).await {
            let _ = save_build_error(&pool, function_id, &format!("Eroare lansare pod builder: {}", e)).await;
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
            let _ = secrets_api.delete("hermes-registry-credentials", &DeleteParams::default()).await;
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

        let mut envs = vec![("ALLOWED_METHOD".to_string(), function_method.clone())];
        if let Some(arr) = env_variables.as_array() {
            for val in arr {
                if let (Some(k), Some(v)) = (val.get("key").and_then(|k| k.as_str()), val.get("value").and_then(|v| v.as_str())) {
                    envs.push((k.to_uppercase(), v.to_string()));
                }
            }
        }

        let deploy_res = K8sManager::deploy_knative_service(
            &k8s_client,
            &namespace,
            &k8s_svc_name,
            &deployment_image,
            envs,
            0, // minScale
            5, // maxScale
            10, // targetConcurrency
            Some(memory_limit_mb)
        ).await;

        if let Err(e) = deploy_res {
            let final_log = format!("{}\n\n=========================================\n STAGE 2: DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!(
                "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
                final_log, function_id
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
        let proxy_res = deploy_proxy_resources(&k8s_client, &namespace, &k8s_svc_name, external_port).await;
        if let Err(e) = proxy_res {
            let final_log = format!("{}\n\n=========================================\n STAGE 3: PROXY DEPLOY FAILED\n=========================================\n{}", total_log, e);
            let _ = sqlx::query!(
                "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
                final_log, function_id
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
            "UPDATE serverless_functions SET status = 'active', build_logs = $1, updated_at = now() WHERE id = $2",
            final_log, function_id
        )
        .execute(&pool)
        .await;

        if let Ok(func) = sqlx::query_as::<_, ServerlessFunction>("SELECT * FROM serverless_functions WHERE id = $1").bind(function_id).fetch_one(&pool).await {
            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::ServerlessFunctionUpdated { workspace_id: ws_id, function: func }
            );
        }
    });

    Ok(StatusCode::ACCEPTED)
}

async fn save_build_error(pool: &sqlx::PgPool, id: Uuid, error_msg: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE serverless_functions SET status = 'failed', build_logs = $1, updated_at = now() WHERE id = $2",
        error_msg, id
    )
    .execute(pool)
    .await?;

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
) -> Result<(), AppError> {
    let configmap_name = format!("{}-proxy-config", ksvc_name);
    let deployment_name = format!("{}-proxy", ksvc_name);
    let service_name = format!("{}-external", ksvc_name);
    let proxy_svc_name = format!("{}-proxy-svc", ksvc_name);

    // 1. ConfigMap for Nginx
    let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    let nginx_conf = format!(
        r#"events {{ worker_connections 1024; }}
http {{
    client_max_body_size 0;
    server {{
        listen 8080;
        location / {{
            proxy_pass http://{}.{}.svc.cluster.local;
            proxy_set_header Host {}.{}.svc.cluster.local;
            proxy_http_version 1.1;
            proxy_set_header Connection "";
            proxy_read_timeout 600s;
            proxy_connect_timeout 600s;
        }}
    }}
}}"#,
        ksvc_name, namespace, ksvc_name, namespace
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
