//! Garbage-collection worker. Reclaims space the platform would otherwise leak:
//! superseded build images in the in-cluster registry (immutable tags mean every
//! build pushes a new one), old `app_builds` records, finished `jobs` rows, and
//! Evicted/Failed pods left behind in workspace namespaces.
//!
//! Leader-gated (only the elected replica runs it) and best-effort: every phase
//! records what it did — or why it couldn't — into a `gc_runs` row that the admin
//! console surfaces (Logs → GC Worker).

use std::time::Duration;
use std::collections::HashSet;
use sqlx::PgPool;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use kube::{Api, api::{DeleteParams, ListParams, PostParams}};
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::batch::v1::Job;
use serde_json::json;

/// Registry repositories the platform pushes to (see utils/builder.rs).
const APP_IMAGE_REPO: &str = "hermes-app-image";
const BUILD_CACHE_REPO: &str = "hermes-build-cache";

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

/// Kaniko's shared layer-cache entries older than this many days are pruned. The cache
/// is fully regenerable (a miss just rebuilds a layer), so bounding it reclaims space
/// with no correctness cost.
fn cache_retention_days() -> i64 {
    std::env::var("HERMES_CACHE_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(3)
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
    let (builds_pruned, mut images_deleted) = prune_old_builds(pool, &mut detail).await;

    // Registry-wide reclamation. A shared client/base for the manifest-level passes.
    let reg_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_default();
    let reg_base = registry_api_base();
    // Tags left behind by deleted apps (no build row references them anymore).
    images_deleted += prune_orphaned_app_images(pool, &reg_client, &reg_base, &mut detail).await;
    // The global Kaniko build cache — unbounded without this.
    images_deleted += prune_build_cache(&reg_client, &reg_base, &mut detail).await;

    let jobs_pruned = prune_old_jobs(pool, &mut detail).await;
    let pods_reaped = reap_failed_pods(&mut detail).await;

    // Unlinking manifests (above) frees NO disk on its own — the registry only reclaims
    // blob storage when `registry garbage-collect` runs. Do it whenever we unlinked
    // anything this pass (and no build is mid-push), or the 20Gi registry PVC fills up.
    if images_deleted > 0 {
        registry_blob_gc(pool, &mut detail).await;
    } else {
        detail.push("registry: no manifests unlinked this pass — blob GC skipped".to_string());
    }

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
            if delete_registry_manifest(&client, &base, APP_IMAGE_REPO, tag).await {
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

/// Accept header covering both the Docker v2 and OCI manifest media types, so digest
/// resolution works regardless of which the pusher (Kaniko / kpack) produced.
const MANIFEST_ACCEPT: &str =
    "application/vnd.docker.distribution.manifest.v2+json, application/vnd.oci.image.manifest.v1+json";

/// Delete one `<repo>:<reference>` manifest from the registry. Resolves the reference
/// to its digest first (the v2 API only deletes by digest). Best-effort — note this
/// only UNLINKS the manifest; blob storage is reclaimed later by [`registry_blob_gc`].
async fn delete_registry_manifest(client: &reqwest::Client, base: &str, repo: &str, reference: &str) -> bool {
    let manifest_url = format!("{}/v2/{}/manifests/{}", base, repo, reference);
    let digest = match client
        .get(&manifest_url)
        .header("Accept", MANIFEST_ACCEPT)
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

    let del_url = format!("{}/v2/{}/manifests/{}", base, repo, digest);
    matches!(client.delete(&del_url).send().await, Ok(r) if r.status().is_success())
}

/// List the tags of a registry repository. Empty when the repo doesn't exist or the
/// registry is unreachable (both are non-fatal for GC).
async fn list_registry_tags(client: &reqwest::Client, base: &str, repo: &str) -> Vec<String> {
    let url = format!("{}/v2/{}/tags/list", base, repo);
    match client.get(&url).send().await {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|j| {
                j.get("tags").and_then(|t| t.as_array()).map(|a| {
                    a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()
                })
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Read an image's `created` timestamp from its config blob (manifest → config digest
/// → blob JSON). Used to age out cache entries. `None` when anything is missing (in
/// which case the caller leaves the tag alone — never delete on ambiguity).
async fn image_created_at(client: &reqwest::Client, base: &str, repo: &str, tag: &str) -> Option<DateTime<Utc>> {
    let manifest_url = format!("{}/v2/{}/manifests/{}", base, repo, tag);
    let manifest: serde_json::Value = client
        .get(&manifest_url)
        .header("Accept", MANIFEST_ACCEPT)
        .send()
        .await
        .ok()
        .filter(|r| r.status().is_success())?
        .json()
        .await
        .ok()?;
    let config_digest = manifest.get("config")?.get("digest")?.as_str()?.to_string();

    let blob_url = format!("{}/v2/{}/blobs/{}", base, repo, config_digest);
    let config: serde_json::Value = client.get(&blob_url).send().await.ok()?.json().await.ok()?;
    let created = config.get("created")?.as_str()?;
    DateTime::parse_from_rfc3339(created).ok().map(|d| d.with_timezone(&Utc))
}

/// Delete `hermes-app-image` tags that no build/instance references anymore — the
/// images left orphaned when an app (and its cascaded `app_builds` rows) is deleted.
/// In-flight builds are protected: their build id IS the tag, and the row exists.
async fn prune_orphaned_app_images(
    pool: &PgPool,
    client: &reqwest::Client,
    base: &str,
    detail: &mut Vec<String>,
) -> i32 {
    let tags = list_registry_tags(client, base, APP_IMAGE_REPO).await;
    if tags.is_empty() {
        return 0;
    }

    // Every tag still referenced by the DB. The kaniko tag == the build id, so guard by
    // both the id and the stored image_tag (covers rows whose tag column isn't set yet).
    let mut live: HashSet<String> = HashSet::new();
    if let Ok(rows) = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT id::text, image_tag FROM app_builds",
    )
    .fetch_all(pool)
    .await
    {
        for (id, image_tag) in rows {
            live.insert(id);
            if let Some(t) = image_tag {
                if let Some(seg) = t.rsplit(':').next() {
                    live.insert(seg.to_string());
                }
            }
        }
    } else {
        detail.push("orphans: could not read app_builds — skipping".to_string());
        return 0;
    }
    if let Ok(rows) = sqlx::query_as::<_, (String,)>(
        "SELECT current_image_tag FROM app_instances WHERE current_image_tag IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    {
        for (t,) in rows {
            if let Some(seg) = t.rsplit(':').next() {
                live.insert(seg.to_string());
            }
        }
    }

    let mut deleted = 0i32;
    for tag in &tags {
        if live.contains(tag) {
            continue;
        }
        if delete_registry_manifest(client, base, APP_IMAGE_REPO, tag).await {
            deleted += 1;
        }
    }
    detail.push(format!(
        "orphans: deleted {} of {} app-image tag(s) with no DB reference",
        deleted, tags.len()
    ));
    deleted
}

/// Prune the shared Kaniko build cache (`hermes-build-cache`): delete cache tags whose
/// image was created before the retention cutoff. Unbounded otherwise — every build
/// with changed dependencies pushes fresh cache layers here forever.
async fn prune_build_cache(client: &reqwest::Client, base: &str, detail: &mut Vec<String>) -> i32 {
    let tags = list_registry_tags(client, base, BUILD_CACHE_REPO).await;
    if tags.is_empty() {
        detail.push("cache: empty / no build cache to prune".to_string());
        return 0;
    }
    let days = cache_retention_days();
    let cutoff = Utc::now() - chrono::Duration::days(days);

    let mut pruned = 0i32;
    let mut inspected = 0usize;
    // Cap work per pass so a huge cache can't stall the GC loop; the rest is caught next run.
    for tag in tags.iter().take(1000) {
        inspected += 1;
        if let Some(created) = image_created_at(client, base, BUILD_CACHE_REPO, tag).await {
            if created < cutoff && delete_registry_manifest(client, base, BUILD_CACHE_REPO, tag).await {
                pruned += 1;
            }
        }
    }
    detail.push(format!(
        "cache: pruned {} of {} inspected cache tag(s) older than {}d",
        pruned, inspected, days
    ));
    pruned
}

/// Reclaim registry blob storage by running `registry garbage-collect` as a one-off
/// Job in kube-system that co-mounts the `registry-data` PVC. Deleting manifests only
/// unlinks them; this is what actually frees disk. Deferred while a build is pushing
/// (GC could race a not-yet-linked blob), and disabled with `HERMES_REGISTRY_BLOB_GC=off`.
async fn registry_blob_gc(pool: &PgPool, detail: &mut Vec<String>) {
    if std::env::var("HERMES_REGISTRY_BLOB_GC")
        .map(|v| v.eq_ignore_ascii_case("off"))
        .unwrap_or(false)
    {
        detail.push("registry: blob GC disabled (HERMES_REGISTRY_BLOB_GC=off)".to_string());
        return;
    }

    // A build mid-push may have uploaded blobs not yet linked to a manifest; GC would
    // delete them. Defer to the next pass rather than risk corrupting an active build.
    let active: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM app_builds WHERE status IN ('building', 'queued')",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    if active > 0 {
        detail.push(format!("registry: {} build(s) in progress — deferring blob GC", active));
        return;
    }

    match run_registry_gc_job().await {
        Ok(exit) => detail.push(format!("registry: blob GC job finished (exit {})", exit)),
        Err(e) => detail.push(format!("registry: blob GC job failed: {}", e)),
    }
}

/// Create + await the `registry garbage-collect -m` Job. Returns the container exit code.
///
/// NOTE: `registry-data` is ReadWriteOnce, so this Job pod co-mounts it on the same node
/// as the registry Deployment — fine on the single-node target; a multi-node deploy would
/// need to pin this Job to the registry's node.
async fn run_registry_gc_job() -> Result<i32, String> {
    let client = crate::utils::k8s::K8sManager::get_client().await.map_err(|e| e.to_string())?;
    let jobs: Api<Job> = Api::namespaced(client.clone(), "kube-system");
    let name = "hermes-registry-gc";

    // Clear any leftover job of the same name (avoids a 409 on create).
    if jobs.get(name).await.is_ok() {
        let dp = DeleteParams {
            propagation_policy: Some(kube::api::PropagationPolicy::Background),
            ..Default::default()
        };
        let _ = jobs.delete(name, &dp).await;
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if jobs.get(name).await.is_err() {
                break;
            }
        }
    }

    let manifest: Job = serde_json::from_value(json!({
        "apiVersion": "batch/v1",
        "kind": "Job",
        "metadata": { "name": name, "namespace": "kube-system" },
        "spec": {
            "backoffLimit": 0,
            "ttlSecondsAfterFinished": 120,
            "activeDeadlineSeconds": 600,
            "template": {
                "spec": {
                    "restartPolicy": "Never",
                    "containers": [{
                        "name": "gc",
                        "image": "registry:2",
                        "imagePullPolicy": "IfNotPresent",
                        // `-m` also removes untagged manifests left after tag deletions.
                        "command": ["/bin/registry", "garbage-collect", "-m", "/etc/docker/registry/config.yml"],
                        "volumeMounts": [{ "name": "data", "mountPath": "/var/lib/registry" }],
                        "resources": {
                            "requests": { "cpu": "50m", "memory": "64Mi" },
                            "limits": { "cpu": "500m", "memory": "256Mi" }
                        }
                    }],
                    "volumes": [{
                        "name": "data",
                        "persistentVolumeClaim": { "claimName": "registry-data" }
                    }]
                }
            }
        }
    }))
    .map_err(|e| format!("job serialization failed: {}", e))?;

    jobs.create(&PostParams::default(), &manifest)
        .await
        .map_err(|e| format!("job create failed: {}", e))?;

    // Await completion (activeDeadlineSeconds bounds the container itself).
    let mut exit = -1;
    for _ in 0..600 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        match jobs.get(name).await {
            Ok(j) => {
                if let Some(s) = j.status {
                    if s.succeeded.unwrap_or(0) > 0 {
                        exit = 0;
                        break;
                    }
                    if s.failed.unwrap_or(0) > 0 {
                        exit = 1;
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }

    let dp = DeleteParams {
        propagation_policy: Some(kube::api::PropagationPolicy::Background),
        ..Default::default()
    };
    let _ = jobs.delete(name, &dp).await;
    Ok(exit)
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
