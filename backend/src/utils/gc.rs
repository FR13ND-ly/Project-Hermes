//! Garbage-collection worker. Reclaims space the platform would otherwise leak:
//! superseded build images in the in-cluster registry (immutable tags mean every
//! build pushes a new one), old `app_builds` records, finished `jobs` rows, and
//! Evicted/Failed pods left behind in workspace namespaces.
//!
//! Leader-gated (only the elected replica runs it) and best-effort: every phase
//! records what it did — or why it couldn't — into a `gc_runs` row that the admin
//! console surfaces (Logs → GC Worker).

use std::time::Duration;
use sqlx::PgPool;
use uuid::Uuid;
use kube::{Api, api::{DeleteParams, ListParams}};
use k8s_openapi::api::core::v1::Pod;

/// Newest builds kept per app; older ones are pruned with their registry image.
fn build_retention() -> i64 {
    std::env::var("HERMES_BUILD_RETENTION")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20)
}

/// Finished jobs older than this many days are deleted.
fn job_retention_days() -> i32 {
    std::env::var("HERMES_JOB_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(7)
}

/// HTTP base for registry API calls from the control plane's vantage point.
/// Defaults to the registry URL the rest of the platform uses.
fn registry_api_base() -> String {
    let raw = std::env::var("HERMES_REGISTRY_API_URL")
        .or_else(|_| std::env::var("HERMES_REGISTRY_URL"))
        .unwrap_or_else(|_| "localhost:5000".to_string());
    let raw = raw.trim().trim_end_matches('/').to_string();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw
    } else {
        format!("http://{}", raw)
    }
}

/// Start the hourly GC loop. Gates each tick on leader election, so with N replicas
/// only one runs it (and with one replica it always runs).
pub fn start_gc_worker(pool: PgPool) {
    tokio::spawn(async move {
        let interval_secs = std::env::var("HERMES_GC_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|n| *n >= 60)
            .unwrap_or(3600);
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        loop {
            interval.tick().await;
            if !crate::utils::leader::is_leader() {
                continue;
            }
            run_gc(&pool).await;
        }
    });
}

/// Run one full GC pass, recording the outcome in `gc_runs`.
async fn run_gc(pool: &PgPool) {
    let started = std::time::Instant::now();
    let run_id = Uuid::new_v4();
    let _ = sqlx::query!("INSERT INTO gc_runs (id, status) VALUES ($1, 'running')", run_id)
        .execute(pool)
        .await;

    let mut detail: Vec<String> = Vec::new();
    let (builds_pruned, images_deleted) = prune_old_builds(pool, &mut detail).await;
    let jobs_pruned = prune_old_jobs(pool, &mut detail).await;
    let pods_reaped = reap_failed_pods(&mut detail).await;

    let duration_ms = started.elapsed().as_millis() as i64;
    let _ = sqlx::query!(
        "UPDATE gc_runs SET finished_at = now(), status = 'success', images_deleted = $2,
                builds_pruned = $3, jobs_pruned = $4, pods_reaped = $5, detail = $6, duration_ms = $7
         WHERE id = $1",
        run_id, images_deleted, builds_pruned, jobs_pruned, pods_reaped, detail.join("\n"), duration_ms
    )
    .execute(pool)
    .await;

    tracing::info!(builds_pruned, images_deleted, jobs_pruned, pods_reaped, duration_ms, "GC pass complete");
}

/// Per app, keep the newest `build_retention()` builds; prune older ones (and their
/// registry image), but never the image currently deployed on any instance.
async fn prune_old_builds(pool: &PgPool, detail: &mut Vec<String>) -> (i32, i32) {
    let keep = build_retention();
    let rows = sqlx::query!(
        r#"
        WITH ranked AS (
            SELECT ab.id, ab.image_tag,
                   row_number() OVER (PARTITION BY ab.app_id ORDER BY ab.created_at DESC) AS rn
            FROM app_builds ab
            WHERE ab.status NOT IN ('building', 'queued')
        )
        SELECT r.id AS "id!", r.image_tag AS "image_tag!"
        FROM ranked r
        WHERE r.rn > $1
          AND r.image_tag IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM app_instances ai WHERE ai.current_image_tag = r.image_tag)
        "#,
        keep
    )
    .fetch_all(pool)
    .await;

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            detail.push(format!("builds: query failed: {}", e));
            return (0, 0);
        }
    };
    if rows.is_empty() {
        detail.push(format!("builds: nothing to prune (keep last {}/app)", keep));
        return (0, 0);
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_default();
    let base = registry_api_base();

    let mut builds_pruned = 0i32;
    let mut images_deleted = 0i32;
    for row in rows {
        // Registry repo is fixed (`hermes-app-image`); the tag is the build id, the
        // segment after the final ':' (host ports contain ':' too, hence rsplit).
        if let Some(tag) = row.image_tag.rsplit(':').next() {
            if delete_registry_image(&client, &base, tag).await {
                images_deleted += 1;
            }
        }
        if sqlx::query!("DELETE FROM app_builds WHERE id = $1", row.id)
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
            .unwrap_or(0)
            > 0
        {
            builds_pruned += 1;
        }
    }
    detail.push(format!(
        "builds: pruned {} record(s), deleted {} image manifest(s) (keep last {}/app)",
        builds_pruned, images_deleted, keep
    ));
    (builds_pruned, images_deleted)
}

/// Delete one `hermes-app-image:<tag>` manifest from the registry. Resolves the tag
/// to its digest first (the v2 API only deletes by digest). Best-effort.
async fn delete_registry_image(client: &reqwest::Client, base: &str, tag: &str) -> bool {
    let manifest_url = format!("{}/v2/hermes-app-image/manifests/{}", base, tag);
    let digest = match client
        .get(&manifest_url)
        .header("Accept", "application/vnd.docker.distribution.manifest.v2+json")
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r
            .headers()
            .get("Docker-Content-Digest")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string()),
        _ => None,
    };
    let Some(digest) = digest else { return false };

    let del_url = format!("{}/v2/hermes-app-image/manifests/{}", base, digest);
    matches!(client.delete(&del_url).send().await, Ok(r) if r.status().is_success())
}

/// Delete finished (succeeded/failed) job rows older than the retention window.
async fn prune_old_jobs(pool: &PgPool, detail: &mut Vec<String>) -> i32 {
    let days = job_retention_days();
    match sqlx::query!(
        "DELETE FROM jobs WHERE status IN ('succeeded', 'failed')
         AND updated_at < now() - ($1::int * interval '1 day')",
        days
    )
    .execute(pool)
    .await
    {
        Ok(r) => {
            let n = r.rows_affected() as i32;
            detail.push(format!("jobs: deleted {} finished row(s) older than {}d", n, days));
            n
        }
        Err(e) => {
            detail.push(format!("jobs: delete failed: {}", e));
            0
        }
    }
}

/// Delete Evicted/Failed pods left behind in workspace namespaces (Kubernetes keeps
/// them around until something reaps them).
async fn reap_failed_pods(detail: &mut Vec<String>) -> i32 {
    let client = match crate::utils::k8s::K8sManager::get_client().await {
        Ok(c) => c,
        Err(e) => {
            detail.push(format!("pods: k8s unavailable: {}", e));
            return 0;
        }
    };
    let all_pods: Api<Pod> = Api::all(client.clone());
    let lp = ListParams::default().fields("status.phase=Failed");
    let list = match all_pods.list(&lp).await {
        Ok(l) => l,
        Err(e) => {
            detail.push(format!("pods: list failed: {}", e));
            return 0;
        }
    };

    let mut reaped = 0i32;
    for p in list.items {
        let ns = p.metadata.namespace.clone().unwrap_or_default();
        let name = p.metadata.name.clone().unwrap_or_default();
        if name.is_empty() || !ns.starts_with("hermes-ws-") {
            continue;
        }
        let ns_pods: Api<Pod> = Api::namespaced(client.clone(), &ns);
        if ns_pods.delete(&name, &DeleteParams::default()).await.is_ok() {
            reaped += 1;
        }
    }
    detail.push(format!("pods: reaped {} Failed/Evicted pod(s)", reaped));
    reaped
}
