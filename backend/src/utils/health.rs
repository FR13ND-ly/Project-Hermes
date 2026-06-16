use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
use reqwest::Client;
use tokio::sync::Semaphore;
use crate::models::app_model::AppStatus;

/// Cap on concurrent in-flight health probes, so a large fleet doesn't burst
/// hundreds of simultaneous requests against the cluster every tick.
const MAX_CONCURRENT_PROBES: usize = 64;
/// Max random delay added before each probe to spread load across the interval
/// (de-synchronizes the "thundering herd" of probes firing on the same tick).
const PROBE_JITTER_MS: u64 = 4000;

pub fn start_health_check_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        let probe_limiter = Arc::new(Semaphore::new(MAX_CONCURRENT_PROBES));

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
                    // Backpressure: block here once MAX_CONCURRENT_PROBES are in
                    // flight instead of spawning unbounded tasks.
                    let Ok(permit) = probe_limiter.clone().acquire_owned().await else { continue; };
                    let client_clone = client.clone();
                    let pool_clone = pool.clone();

                    let target_url = format!(
                        "http://{}:{}{}",
                        inst.container_name,
                        inst.internal_port,
                        inst.health_check_path.unwrap_or_else(|| "/".to_string())
                    );

                    tokio::spawn(async move {
                        let _permit = permit; // released when the probe finishes
                        // Jitter so probes don't all fire on the same instant.
                        tokio::time::sleep(Duration::from_millis(rand::random::<u64>() % PROBE_JITTER_MS)).await;
                        let response = client_clone.get(&target_url).send().await;

                        match response {
                            Ok(res) => {
                                if res.status().is_success() {
                                    crate::utils::metrics::record_health_check("ok");
                                    let _ = resolve_active_incidents(&pool_clone, inst.id).await;
                                } else {
                                    crate::utils::metrics::record_health_check("unhealthy");
                                    let msg = format!("App returned unhealthy status code: {}", res.status());
                                    let _ = record_health_failure(&pool_clone, inst.workspace_id, inst.id, "UNHEALTHY_HTTP_CODE", &msg).await;
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
                                    crate::utils::metrics::record_health_check("ok");
                                    let _ = resolve_active_incidents(&pool_clone, inst.id).await;
                                } else {
                                    crate::utils::metrics::record_health_check("unreachable");
                                    let msg = format!("Failed to reach application endpoint. Error: {}", e);
                                    let _ = record_health_failure(&pool_clone, inst.workspace_id, inst.id, "TIMEOUT_OR_DOWN", &msg).await;
                                }
                            }
                        }
                    });
                }
            }
        }
    });
}

/// Periodically check resource-saturation metrics from Prometheus and raise
/// incidents for conditions a plain HTTP 200 would hide (sustained memory
/// pressure). Reuses the same incident table + alert webhooks as the HTTP probe,
/// but manages its own raise/resolve lifecycle and never flips the app's status.
pub fn start_metric_alert_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        let mem_threshold = std::env::var("HERMES_MEM_ALERT_RATIO")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|r| *r > 0.0 && *r <= 1.0)
            .unwrap_or(0.9);

        loop {
            interval.tick().await;

            let instances = sqlx::query!(
                "SELECT ai.id, ai.container_name, ai.memory_limit_mb, a.workspace_id
                 FROM app_instances ai JOIN apps a ON ai.app_id = a.id
                 WHERE ai.status = 'running' AND ai.memory_limit_mb > 0"
            )
            .fetch_all(&pool)
            .await;

            let Ok(instances) = instances else { continue; };

            for inst in instances {
                let limit_mb = inst.memory_limit_mb.unwrap_or(0);
                if limit_mb <= 0 {
                    continue;
                }
                let namespace = format!("hermes-ws-{}", inst.workspace_id);
                let promql = format!(
                    "sum(container_memory_working_set_bytes{{namespace=\"{}\",pod=~\"{}-.*\"}})",
                    namespace, inst.container_name
                );

                let Some(used_bytes) = crate::utils::prometheus::query_instant(&promql).await else {
                    continue;
                };
                let limit_bytes = limit_mb as f64 * 1024.0 * 1024.0;
                let ratio = used_bytes / limit_bytes;

                let open = sqlx::query_scalar!(
                    "SELECT EXISTS(SELECT 1 FROM app_incident_logs WHERE app_instance_id = $1 AND incident_type = 'HIGH_MEMORY' AND resolved_at IS NULL)",
                    inst.id
                )
                .fetch_one(&pool)
                .await
                .ok()
                .flatten()
                .unwrap_or(false);

                if ratio >= mem_threshold && !open {
                    let pct = (ratio * 100.0).round();
                    let msg = format!("Memory usage at {}% of the {}MB limit.", pct, limit_mb);
                    let incident_id = Uuid::new_v4();
                    let _ = sqlx::query!(
                        "INSERT INTO app_incident_logs (id, workspace_id, app_instance_id, incident_type, message) VALUES ($1, $2, $3, 'HIGH_MEMORY', $4)",
                        incident_id, inst.workspace_id, inst.id, msg
                    )
                    .execute(&pool)
                    .await;
                    dispatch_external_alert(&pool, inst.workspace_id, inst.id, "HIGH_MEMORY", &msg).await;
                } else if ratio < mem_threshold && open {
                    // Recovered — resolve the open memory incident.
                    let _ = sqlx::query!(
                        "UPDATE app_incident_logs SET resolved_at = now() WHERE app_instance_id = $1 AND incident_type = 'HIGH_MEMORY' AND resolved_at IS NULL",
                        inst.id
                    )
                    .execute(&pool)
                    .await;
                }
            }
        }
    });
}

/// Count a failed health check. An incident (status flip + alerts) is only raised
/// once `consecutive_health_failures` reaches the threshold, so a single transient
/// blip never marks an app down or pages anyone.
async fn record_health_failure(pool: &PgPool, workspace_id: Uuid, instance_id: Uuid, incident_type: &str, message: &str) -> Result<(), sqlx::Error> {
    let threshold: i32 = std::env::var("HERMES_HEALTH_FAILURE_THRESHOLD")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(3);

    let count = sqlx::query_scalar!(
        "UPDATE app_instances SET consecutive_health_failures = consecutive_health_failures + 1, updated_at = now()
         WHERE id = $1 RETURNING consecutive_health_failures",
        instance_id
    )
    .fetch_one(pool)
    .await?;

    if count >= threshold {
        trigger_incident(pool, workspace_id, instance_id, incident_type, message).await?;
    } else {
        tracing::warn!(
            instance_id = %instance_id,
            failures = count,
            threshold,
            incident_type,
            "Health check failed (below threshold — not yet alerting)"
        );
    }
    Ok(())
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
    // A healthy check clears the failure streak so the threshold restarts cleanly.
    let _ = sqlx::query!(
        "UPDATE app_instances SET consecutive_health_failures = 0
         WHERE id = $1 AND consecutive_health_failures <> 0",
        instance_id
    )
    .execute(pool)
    .await;

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
        tracing::info!(instance_id = %instance_id, "Instance has recovered successfully");
    }

    Ok(())
}

async fn dispatch_external_alert(pool: &PgPool, workspace_id: Uuid, instance_id: Uuid, incident_type: &str, message: &str) {
    tracing::error!(
        %workspace_id, %instance_id, incident_type, message,
        "CRITICAL: instance health incident raised"
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
                let result = client.post(&url)
                    .json(&payload)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await;
                let outcome = match result {
                    Ok(res) if res.status().is_success() => "success",
                    _ => "failed",
                };
                crate::utils::metrics::record_webhook(&wtype, outcome);
            });
        }
    }
}