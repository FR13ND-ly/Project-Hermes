//! Chaos simulator: controlled, auto-reverting fault injection on app instances.
//!
//! Faults are applied via the same kube-rs primitives the platform already uses
//! (`delete_pod`, `scale_deployment`, `exec_in_pod`). Durable state lives in the
//! `chaos_experiments` table; a leader-gated worker reverts experiments whose
//! `revert_at` has elapsed (and reclaims any left `running` across a restart). The
//! reconcile loop skips replica convergence while an experiment is `running` so it
//! never fights a deliberate scale-down.

use std::time::Duration;
use sqlx::PgPool;
use uuid::Uuid;
use chrono::Utc;

use crate::dtos::chaos_dto::{ChaosExperimentResponse, StartChaosRequest};
use crate::utils::error::AppError;
use crate::utils::k8s::K8sManager;

const MIN_DURATION_SEC: i64 = 5;
const MAX_DURATION_SEC: i64 = 300;

fn clamp_duration(secs: Option<i64>) -> i64 {
    secs.unwrap_or(60).clamp(MIN_DURATION_SEC, MAX_DURATION_SEC)
}

/// Broadcast a refresh nudge so open Networking/Overview tabs re-poll.
fn nudge(workspace_id: Uuid, instance_id: Uuid, container_name: &str, status: &str) {
    crate::utils::event_broadcaster::broadcast_event(
        crate::utils::event_broadcaster::SystemEvent::InstanceStatusChanged {
            workspace_id,
            instance_id,
            container_name: container_name.to_string(),
            status: status.to_string(),
        },
    );
}

/// Open a CHAOS incident so the failure is visible in the incident feed (and so the
/// health worker doesn't also log a duplicate). Does NOT flip the instance status —
/// the app isn't really broken, we're testing it.
async fn open_incident(pool: &PgPool, workspace_id: Uuid, instance_id: Uuid, kind: &str, message: &str) {
    let _ = sqlx::query!(
        "INSERT INTO app_incident_logs (id, workspace_id, app_instance_id, incident_type, message)
         VALUES ($1, $2, $3, $4, $5)",
        Uuid::new_v4(),
        workspace_id,
        instance_id,
        format!("CHAOS_{}", kind.to_uppercase()),
        message
    )
    .execute(pool)
    .await;
}

async fn resolve_chaos_incidents(pool: &PgPool, instance_id: Uuid) {
    let _ = sqlx::query!(
        "UPDATE app_incident_logs SET resolved_at = now()
         WHERE app_instance_id = $1 AND incident_type LIKE 'CHAOS\\_%' AND resolved_at IS NULL",
        instance_id
    )
    .execute(pool)
    .await;
}

/// Apply a chaos fault and record the experiment. Returns the created experiment.
#[allow(clippy::too_many_arguments)]
pub async fn start_experiment(
    pool: &PgPool,
    workspace_id: Uuid,
    app_id: Uuid,
    instance_id: Uuid,
    container_name: &str,
    replicas_min: i32,
    created_by: Uuid,
    req: &StartChaosRequest,
) -> Result<ChaosExperimentResponse, AppError> {
    let client = K8sManager::get_client().await?;
    let namespace = format!("hermes-ws-{}", workspace_id);

    let id = Uuid::new_v4();
    let started_at = Utc::now();

    // Per-kind: apply the fault, then decide status / revert_at / original_replicas.
    let (status, revert_at, original_replicas, message, params) = match req.kind.as_str() {
        // Instantaneous: delete pod(s); the Deployment reschedules. Nothing to revert.
        "pod_kill" => {
            let all = req.target_all_pods.unwrap_or(false);
            let pods: kube::Api<k8s_openapi::api::core::v1::Pod> =
                kube::Api::namespaced(client.clone(), &namespace);
            let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));
            let names: Vec<String> = pods
                .list(&lp)
                .await
                .map(|l| l.items.into_iter().filter_map(|p| p.metadata.name).collect())
                .unwrap_or_default();
            if names.is_empty() {
                return Err(AppError::Validation("No running pods to kill for this instance.".into()));
            }
            let targets: Vec<String> = if all { names } else { names.into_iter().take(1).collect() };
            for n in &targets {
                let _ = K8sManager::delete_pod(&client, &namespace, n).await;
            }
            (
                "completed".to_string(),
                None,
                None,
                format!("Killed {} pod(s)", targets.len()),
                serde_json::json!({ "killed": targets, "all": all }),
            )
        }
        // Drop replicas for a window, then auto-restore to replicas_min.
        "scale_down" => {
            let target = req.target_replicas.unwrap_or(0).max(0);
            let dur = clamp_duration(req.duration_sec);
            K8sManager::scale_deployment(&client, &namespace, container_name, target).await?;
            let msg = format!("Scaled down to {} replica(s) for {}s (restores to {})", target, dur, replicas_min);
            open_incident(pool, workspace_id, instance_id, &req.kind, &msg).await;
            (
                "running".to_string(),
                Some(started_at + chrono::Duration::seconds(dur)),
                Some(replicas_min),
                msg,
                serde_json::json!({ "target_replicas": target, "duration_sec": dur }),
            )
        }
        // Best-effort in-pod CPU burn (needs a shell in the image). Self-terminates
        // via `timeout`, so the revert is just closing the record.
        "cpu_stress" => {
            let dur = clamp_duration(req.duration_sec);
            let workers = req.cpu_workers.unwrap_or(1).clamp(1, 8);
            let pod = K8sManager::pod_name_for_app(&client, &namespace, container_name).await?;
            let burn = format!(
                "timeout {dur} sh -c 'n={workers}; i=0; while [ $i -lt $n ]; do (while true; do :; done) & i=$((i+1)); done; wait' 2>/dev/null || true",
                dur = dur, workers = workers
            );
            // Fire-and-forget: exec blocks for `dur`, so don't hold the request.
            let c = client.clone();
            let ns = namespace.clone();
            tokio::spawn(async move {
                let _ = K8sManager::exec_in_pod(&c, &ns, &pod, vec!["sh".into(), "-c".into(), burn]).await;
            });
            let msg = format!("CPU stress: {} worker(s) for {}s (best-effort)", workers, dur);
            open_incident(pool, workspace_id, instance_id, &req.kind, &msg).await;
            (
                "running".to_string(),
                Some(started_at + chrono::Duration::seconds(dur)),
                None,
                msg,
                serde_json::json!({ "cpu_workers": workers, "duration_sec": dur }),
            )
        }
        other => return Err(AppError::Validation(format!("Unknown chaos kind '{}'.", other))),
    };

    let ended_at = if status == "completed" { Some(Utc::now()) } else { None };

    sqlx::query!(
        "INSERT INTO chaos_experiments
            (id, workspace_id, app_id, app_instance_id, kind, params, status, original_replicas, message, started_at, revert_at, ended_at, created_by)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
        id, workspace_id, app_id, instance_id, req.kind, params, status, original_replicas, message, started_at, revert_at, ended_at, created_by
    )
    .execute(pool)
    .await
    .map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;

    // The platform-level status is unchanged (we don't flip the DB row); this is just
    // a refresh nudge so open Networking/Overview tabs re-poll and show the impact.
    nudge(workspace_id, instance_id, container_name, "running");

    Ok(ChaosExperimentResponse {
        id,
        kind: req.kind.clone(),
        status,
        message: Some(message),
        params,
        original_replicas,
        started_at,
        revert_at,
        ended_at,
    })
}

/// Revert a running experiment: restore state, close the record, resolve incidents.
/// `final_status` is "completed" (auto/normal) or "cancelled" (user stop).
pub async fn revert_experiment(pool: &PgPool, exp_id: Uuid, final_status: &str) {
    let exp = sqlx::query!(
        "SELECT ce.workspace_id, ce.app_instance_id, ce.kind, ce.original_replicas, ai.container_name
         FROM chaos_experiments ce
         JOIN app_instances ai ON ce.app_instance_id = ai.id
         WHERE ce.id = $1 AND ce.status = 'running'",
        exp_id
    )
    .fetch_optional(pool)
    .await;

    let Ok(Some(e)) = exp else { return };

    // Restore replicas for scale_down (pod_kill/cpu_stress self-recover).
    if e.kind == "scale_down" {
        if let Some(target) = e.original_replicas {
            if let Ok(client) = K8sManager::get_client().await {
                let namespace = format!("hermes-ws-{}", e.workspace_id);
                let _ = K8sManager::scale_deployment(&client, &namespace, &e.container_name, target).await;
            }
        }
    }

    let _ = sqlx::query!(
        "UPDATE chaos_experiments SET status = $1, ended_at = now() WHERE id = $2",
        final_status, exp_id
    )
    .execute(pool)
    .await;

    resolve_chaos_incidents(pool, e.app_instance_id).await;
    nudge(e.workspace_id, e.app_instance_id, &e.container_name, "running");
}

/// True if an experiment is currently active on this instance (reconcile guard).
pub async fn has_active_experiment(pool: &PgPool, instance_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM chaos_experiments WHERE app_instance_id = $1 AND status = 'running')",
    )
    .bind(instance_id)
    .fetch_one(pool)
    .await
    .unwrap_or(false)
}

/// Leader-gated worker: auto-revert experiments whose window has elapsed. Also
/// reclaims experiments left `running` after a restart (their `revert_at` is in the
/// past, so they're picked up on the next tick).
pub fn start_chaos_worker(pool: PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            if !crate::utils::leader::is_leader() {
                continue;
            }
            let due = sqlx::query!(
                "SELECT id FROM chaos_experiments
                 WHERE status = 'running' AND revert_at IS NOT NULL AND revert_at <= now()"
            )
            .fetch_all(&pool)
            .await;
            if let Ok(rows) = due {
                for r in rows {
                    revert_experiment(&pool, r.id, "completed").await;
                }
            }
        }
    });
}
