use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;
use reqwest::Client;
use crate::models::app_model::AppStatus;

pub fn start_health_check_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();

        loop {
            interval.tick().await;

            let active_instances = sqlx::query!(
                "SELECT ai.id, ai.container_name, ai.internal_port, ai.health_check_path, ai.status as \"status: AppStatus\", a.workspace_id 
                 FROM app_instances ai
                 JOIN apps a ON ai.app_id = a.id
                 WHERE ai.status = 'running' OR ai.status = 'failed' OR ai.status = 'crashed'"
            )
            .fetch_all(&pool)
            .await;

            if let Ok(instances) = active_instances {
                for inst in instances {
                    let client_clone = client.clone();
                    let pool_clone = pool.clone();
                    
                    let target_url = format!(
                        "http://{}:{}{}",
                        inst.container_name,
                        inst.internal_port,
                        inst.health_check_path.unwrap_or_else(|| "/".to_string())
                    );

                    tokio::spawn(async move {
                        let response = client_clone.get(&target_url).send().await;

                        match response {
                            Ok(res) => {
                                if res.status().is_success() {
                                    let _ = resolve_active_incidents(&pool_clone, inst.id).await;
                                } else {
                                    let msg = format!("App returned unhealthy status code: {}", res.status());
                                    let _ = trigger_incident(&pool_clone, inst.workspace_id, inst.id, "UNHEALTHY_HTTP_CODE", &msg).await;
                                }
                            }
                            Err(e) => {
                                // If we are running outside the cluster (e.g. local Windows development),
                                // we cannot resolve or reach internal cluster DNS names directly.
                                // In this case, we check if the Deployment is healthy in Kubernetes.
                                let mut is_healthy = false;
                                if std::env::var("KUBERNETES_SERVICE_HOST").is_err() {
                                    if let Ok(k8s_client) = crate::utils::k8s::K8sManager::get_client().await {
                                        let namespace = format!("hermes-ws-{}", inst.workspace_id);
                                        let deployments: kube::Api<k8s_openapi::api::apps::v1::Deployment> = kube::Api::namespaced(k8s_client, &namespace);
                                        if let Ok(deploy) = deployments.get(&inst.container_name).await {
                                            if let Some(status) = deploy.status {
                                                if status.ready_replicas.unwrap_or(0) > 0 {
                                                    is_healthy = true;
                                                }
                                            }
                                        }
                                    }
                                }

                                if is_healthy {
                                    let _ = resolve_active_incidents(&pool_clone, inst.id).await;
                                } else {
                                    let msg = format!("Failed to reach application endpoint. Error: {}", e);
                                    let _ = trigger_incident(&pool_clone, inst.workspace_id, inst.id, "TIMEOUT_OR_DOWN", &msg).await;
                                }
                            }
                        }
                    });
                }
            }
        }
    });
}

async fn trigger_incident(pool: &PgPool, workspace_id: Uuid, instance_id: Uuid, incident_type: &str, message: &str) -> Result<(), sqlx::Error> {
    let already_logged = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM app_incident_logs WHERE app_instance_id = $1 AND resolved_at IS NULL)",
        instance_id
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !already_logged {
        let incident_id = Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO app_incident_logs (id, workspace_id, app_instance_id, incident_type, message) 
             VALUES ($1, $2, $3, $4, $5)",
            incident_id, workspace_id, instance_id, incident_type, message
        )
        .execute(pool)
        .await?;

        sqlx::query!(
            "UPDATE app_instances SET status = 'failed', updated_at = now() WHERE id = $1",
            instance_id
        )
        .execute(pool)
        .await?;

        if let Ok(Some(meta)) = sqlx::query!(
            "SELECT a.project_id, ai.container_name FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1",
            instance_id
        )
        .fetch_optional(pool)
        .await {
            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::IncidentCreated {
                    workspace_id,
                    incident_id,
                    project_id: meta.project_id,
                    message: message.to_string(),
                }
            );

            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                    workspace_id,
                    instance_id,
                    container_name: meta.container_name,
                    status: "failed".to_string(),
                }
            );
        }

        dispatch_external_alert(pool, workspace_id, instance_id, incident_type, message).await;
    }

    Ok(())
}

async fn resolve_active_incidents(pool: &PgPool, instance_id: Uuid) -> Result<(), sqlx::Error> {
    let rows = sqlx::query!(
        "UPDATE app_incident_logs SET resolved_at = now() WHERE app_instance_id = $1 AND resolved_at IS NULL",
        instance_id
    )
    .execute(pool)
    .await?;

    // Also ensure status is running if the check succeeded
    sqlx::query!(
        "UPDATE app_instances SET status = 'running', updated_at = now() WHERE id = $1 AND status != 'running'",
        instance_id
    )
    .execute(pool)
    .await?;

    if rows.rows_affected() > 0 {
        if let Ok(Some(meta)) = sqlx::query!(
            "SELECT a.workspace_id, ai.container_name FROM app_instances ai JOIN apps a ON ai.app_id = a.id WHERE ai.id = $1",
            instance_id
        )
        .fetch_optional(pool)
        .await {
            crate::utils::event_broadcaster::broadcast_event(
                crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
                    workspace_id: meta.workspace_id,
                    instance_id,
                    container_name: meta.container_name,
                    status: "running".to_string(),
                }
            );
        }
        println!("[Hermes Health] Instance {} has recovered successfully.", instance_id);
    }

    Ok(())
}

async fn dispatch_external_alert(pool: &PgPool, workspace_id: Uuid, instance_id: Uuid, incident_type: &str, message: &str) {
    println!(
        "[ALERT SYSTEM] CRITICAL ERROR in Workspace {} | Instance {} | Type: {} | Message: {}",
        workspace_id, instance_id, incident_type, message
    );
    
    let webhooks = sqlx::query!(
        "SELECT w.url, w.webhook_type
         FROM project_webhooks w
         JOIN apps a ON w.project_id = a.project_id
         JOIN app_instances ai ON a.id = ai.app_id
         WHERE ai.id = $1 AND w.is_active = true",
        instance_id
    )
    .fetch_all(pool)
    .await;

    if let Ok(webhooks) = webhooks {
        if webhooks.is_empty() {
            return;
        }

        let client = reqwest::Client::new();
        let timestamp = chrono::Utc::now().to_rfc3339();

        for wh in webhooks {
            let client = client.clone();
            let url = wh.url.clone();
            let wtype = wh.webhook_type.clone();

            let payload = match wtype.as_str() {
                "discord" => {
                    serde_json::json!({
                        "embeds": [
                            {
                                "title": "🚨 ALERTĂ INCIDENT - Hermes Orchestrator",
                                "description": "S-a înregistrat o problemă în starea de funcționare a aplicației.",
                                "color": 15158332,
                                "fields": [
                                    {
                                        "name": "ID Instanță",
                                        "value": format!("`{}`", instance_id),
                                        "inline": true
                                    },
                                    {
                                        "name": "Tip Incident",
                                        "value": format!("`{}`", incident_type),
                                        "inline": true
                                    },
                                    {
                                        "name": "Mesaj",
                                        "value": message
                                    }
                                ],
                                "timestamp": timestamp
                            }
                        ]
                    })
                }
                "slack" => {
                    serde_json::json!({
                        "text": format!("🚨 *[ALERTĂ INCIDENT]*\n*Instanță:* `{}`\n*Tip:* `{}`\n*Mesaj:* {}\n*Timp:* {}", instance_id, incident_type, message, timestamp)
                    })
                }
                _ => {
                    serde_json::json!({
                        "workspaceId": workspace_id,
                        "instanceId": instance_id,
                        "incidentType": incident_type,
                        "message": message,
                        "timestamp": timestamp
                    })
                }
            };

            tokio::spawn(async move {
                let _ = client.post(&url)
                    .json(&payload)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await;
            });
        }
    }
}