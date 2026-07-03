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

/// Sentinel the burn workers poll — touching it stops every worker within
/// milliseconds. Lives in /tmp so it also works on read-only root filesystems
/// that mount a writable /tmp; each new burn removes it before starting.
const CHAOS_STOP_FILE: &str = "/tmp/.hermes-chaos-stop";

fn clamp_duration(secs: Option<i64>) -> i64 {
    secs.unwrap_or(60).clamp(MIN_DURATION_SEC, MAX_DURATION_SEC)
}

/// POSIX-sh CPU burn that actually terminates. The previous
/// `timeout N sh -c '(while true …)&'` variant killed only the parent shell on
/// timeout/cancel — the background busy-loops were reparented to PID 1 and kept
/// spinning forever, eating the node's CPU. Here each worker re-checks (every
/// ~20k arithmetic iterations, i.e. every few ms) both its own deadline and the
/// stop sentinel, so it exits on time by itself and instantly when a revert
/// touches [`CHAOS_STOP_FILE`].
fn burn_script(dur: i64, workers: i32) -> String {
    format!(
        "S={stop}; rm -f $S; end=$(($(date +%s)+{dur})); i=0; \
         while [ $i -lt {workers} ]; do \
         (c=0; while :; do c=$((c+1)); if [ $c -ge 20000 ]; then c=0; \
         if [ -f $S ] || [ $(date +%s) -ge $end ]; then break; fi; fi; done) & \
         i=$((i+1)); done; wait",
        stop = CHAOS_STOP_FILE,
        dur = dur,
        workers = workers
    )
}

/// Non-terminating pods behind an app deployment (full objects, for spec checks).
async fn list_app_pods(
    client: &kube::Client,
    namespace: &str,
    container_name: &str,
) -> Vec<k8s_openapi::api::core::v1::Pod> {
    let pods: kube::Api<k8s_openapi::api::core::v1::Pod> =
        kube::Api::namespaced(client.clone(), namespace);
    let lp = kube::api::ListParams::default().labels(&format!("app={}", container_name));
    pods.list(&lp)
        .await
        .map(|l| {
            l.items
                .into_iter()
                .filter(|p| p.metadata.deletion_timestamp.is_none())
                .collect()
        })
        .unwrap_or_default()
}

/// Touch the stop sentinel in every pod so burn workers exit now. Best-effort:
/// pods that died or images without `sh` are skipped silently.
async fn stop_burn(client: &kube::Client, namespace: &str, container_name: &str) {
    let cmd = format!("touch {}", CHAOS_STOP_FILE);
    for pod in list_app_pods(client, namespace, container_name).await {
        if let Some(name) = pod.metadata.name {
            let _ = K8sManager::exec_in_pod(
                client,
                namespace,
                &name,
                vec!["sh".into(), "-c".into(), cmd.clone()],
            )
            .await;
        }
    }
}

/// True when at least one container in the pod declares a CPU *request* —
/// without one the HPA cannot compute utilization and will never scale up.
fn pod_has_cpu_request(pod: &k8s_openapi::api::core::v1::Pod) -> bool {
    pod.spec
        .as_ref()
        .map(|s| {
            s.containers.iter().any(|c| {
                c.resources
                    .as_ref()
                    .and_then(|r| r.requests.as_ref())
                    .map(|req| req.contains_key("cpu"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
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

/// The instance's replica/autoscale settings, needed to validate and describe
/// scaling-related experiments.
pub struct ScalingContext {
    pub replicas_min: i32,
    pub replicas_max: i32,
    pub autoscale_cpu_percent: i32,
}

/// Apply a chaos fault and record the experiment. Returns the created experiment.
#[allow(clippy::too_many_arguments)]
pub async fn start_experiment(
    pool: &PgPool,
    workspace_id: Uuid,
    app_id: Uuid,
    instance_id: Uuid,
    container_name: &str,
    scaling: ScalingContext,
    created_by: Uuid,
    req: &StartChaosRequest,
) -> Result<ChaosExperimentResponse, AppError> {
    let replicas_min = scaling.replicas_min;
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
                .map(|l| {
                    l.items
                        .into_iter()
                        .filter(|p| p.metadata.deletion_timestamp.is_none())
                        .filter_map(|p| p.metadata.name)
                        .collect()
                })
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
        // Best-effort in-pod CPU burn (needs a shell in the image). Workers
        // self-terminate at the deadline and honour the stop sentinel, so a
        // manual revert genuinely kills them (see `burn_script`).
        "cpu_stress" => {
            let dur = clamp_duration(req.duration_sec);
            let workers = req.cpu_workers.unwrap_or(1).clamp(1, 8);
            let pod = K8sManager::pod_name_for_app(&client, &namespace, container_name).await?;
            let burn = burn_script(dur, workers);
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
        // Burn CPU on EVERY pod so the HPA's *average* utilization crosses its target
        // and it schedules extra pod(s) — a live demo that autoscaling actually works.
        // New pods arrive idle, which pulls the average back down: exactly the
        // scale-up-then-stabilize curve the user should see. Nothing to revert — the
        // burn self-terminates and the HPA scales back after its stabilization window.
        "autoscale_burst" => {
            if scaling.replicas_max <= scaling.replicas_min {
                return Err(AppError::Validation(
                    "Autoscaling is disabled for this instance — set max replicas above min in Settings → Scaling so the HPA can add pods.".into(),
                ));
            }
            // Longer default than other kinds: metrics-server scrape + HPA sync need
            // ~30–90s before the new pod appears, so leave time to watch it happen.
            let dur = req.duration_sec.unwrap_or(240).clamp(MIN_DURATION_SEC, MAX_DURATION_SEC);
            let workers = req.cpu_workers.unwrap_or(4).clamp(1, 8);
            let pod_items = list_app_pods(&client, &namespace, container_name).await;
            if pod_items.is_empty() {
                return Err(AppError::Validation("No running pods to stress for this instance.".into()));
            }
            // The HPA scales on utilization = usage / *request*. Hermes only sets a
            // CPU request when the instance has a CPU limit, so with an uncapped
            // instance the burn maxes the node while the HPA sits blind and no pod
            // ever appears. Refuse loudly instead of running a broken demo.
            if !pod_items.iter().any(pod_has_cpu_request) {
                return Err(AppError::Validation(
                    "The pods have no CPU request, so the HPA cannot measure utilization and will never scale up. Set a CPU limit for this instance (Settings → Resources) and redeploy, then run the burst again.".into(),
                ));
            }
            let names: Vec<String> = pod_items.into_iter().filter_map(|p| p.metadata.name).collect();
            // Quick blocking probe: without a shell in the image the burn execs
            // would all fail silently and the experiment would "run" doing nothing.
            if let Some(first) = names.first() {
                K8sManager::exec_in_pod(&client, &namespace, first, vec!["sh".into(), "-c".into(), ":".into()])
                    .await
                    .map_err(|_| AppError::Validation(
                        "The container image has no usable shell (`sh`), so the CPU burn cannot run in this app.".into(),
                    ))?;
            }
            let burn = burn_script(dur, workers);
            // Fire-and-forget per pod: each exec blocks for `dur`, so don't hold the request.
            for pod in &names {
                let c = client.clone();
                let ns = namespace.clone();
                let pod = pod.clone();
                let cmd = burn.clone();
                tokio::spawn(async move {
                    let _ = K8sManager::exec_in_pod(&c, &ns, &pod, vec!["sh".into(), "-c".into(), cmd]).await;
                });
            }
            let msg = format!(
                "Autoscale burst: {} CPU worker(s) on {} pod(s) for {}s — the HPA adds pods once average CPU passes {}% (max {} replicas)",
                workers, names.len(), dur, scaling.autoscale_cpu_percent, scaling.replicas_max
            );
            open_incident(pool, workspace_id, instance_id, &req.kind, &msg).await;
            (
                "running".to_string(),
                Some(started_at + chrono::Duration::seconds(dur)),
                None,
                msg,
                serde_json::json!({
                    "cpu_workers": workers,
                    "duration_sec": dur,
                    "stressed_pods": names.len(),
                    "hpa_cpu_target": scaling.autoscale_cpu_percent,
                    "replicas_max": scaling.replicas_max,
                }),
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

    // Restore replicas for scale_down (pod_kill self-recovers).
    if e.kind == "scale_down" {
        if let Some(target) = e.original_replicas {
            if let Ok(client) = K8sManager::get_client().await {
                let namespace = format!("hermes-ws-{}", e.workspace_id);
                let _ = K8sManager::scale_deployment(&client, &namespace, &e.container_name, target).await;
            }
        }
    }

    // Kill burn workers for the CPU experiments: touch the stop sentinel every
    // worker polls. Runs on manual cancel AND on the auto-revert tick, so even a
    // pod whose clock drifted past the deadline gets cleaned up.
    if e.kind == "cpu_stress" || e.kind == "autoscale_burst" {
        if let Ok(client) = K8sManager::get_client().await {
            let namespace = format!("hermes-ws-{}", e.workspace_id);
            stop_burn(&client, &namespace, &e.container_name).await;
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
