//! Durable background-job queue for build/deploy work.
//!
//! Replaces fire-and-forget `tokio::spawn`: jobs are rows in `jobs`, claimed by
//! workers with `FOR UPDATE SKIP LOCKED`, heartbeated while running, and retried
//! with backoff. A worker that dies mid-job (process restart) leaves a stale
//! heartbeat; `reclaim_stale` requeues it — so build/deploy work is no longer
//! lost on restart (the root cause of the "stuck deploying" class of bugs).

use std::time::Duration;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

use crate::utils::error::AppError;

/// A job no worker has heartbeated within this window is presumed orphaned
/// (its worker crashed) and is requeued. Must exceed the heartbeat interval by a
/// wide margin so a *running* long build is never reclaimed out from under itself.
const STALE_AFTER: Duration = Duration::from_secs(180);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(30);

#[derive(Debug, Serialize, Deserialize)]
struct BuildPayload {
    instance_id: Uuid,
    git_repo: String,
    branch: String,
    build_cmd: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DeployPayload {
    instance_id: Uuid,
    /// Explicit image tag (rollback). `None` => deploy the instance's latest tag.
    image_tag: Option<String>,
}

/// Exponential backoff (seconds) before retrying a failed job, capped at 5 min.
fn backoff_secs(attempts: i32) -> i64 {
    let base = 10_i64.saturating_mul(2_i64.saturating_pow(attempts.max(0) as u32));
    base.min(300)
}

async fn enqueue(pool: &PgPool, kind: &str, payload: serde_json::Value) -> Result<(), AppError> {
    sqlx::query!(
        "INSERT INTO jobs (id, kind, payload) VALUES ($1, $2, $3)",
        Uuid::new_v4(),
        kind,
        payload
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Enqueue a build (clone → image build → deploy) for an app instance.
pub async fn enqueue_build(
    pool: &PgPool,
    instance_id: Uuid,
    git_repo: String,
    branch: String,
    build_cmd: Option<String>,
) -> Result<(), AppError> {
    let payload = serde_json::to_value(BuildPayload { instance_id, git_repo, branch, build_cmd })
        .map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
    enqueue(pool, "build", payload).await
}

/// Enqueue a (re)deploy of an app instance. `image_tag = None` deploys the
/// instance's current/latest image; `Some(tag)` deploys a specific one (rollback).
pub async fn enqueue_deploy(
    pool: &PgPool,
    instance_id: Uuid,
    image_tag: Option<String>,
) -> Result<(), AppError> {
    let payload = serde_json::to_value(DeployPayload { instance_id, image_tag })
        .map_err(|e| AppError::Fatal(anyhow::anyhow!(e)))?;
    enqueue(pool, "deploy", payload).await
}

struct ClaimedJob {
    id: Uuid,
    kind: String,
    payload: serde_json::Value,
}

/// Atomically claim the next runnable job (oldest first), skipping rows locked by
/// other workers. Marks it 'running' and stamps the heartbeat.
async fn claim_next(pool: &PgPool, worker_id: &str) -> Result<Option<ClaimedJob>, AppError> {
    let row = sqlx::query!(
        r#"UPDATE jobs SET status='running', attempts = attempts + 1,
                  locked_at = now(), locked_by = $1, updated_at = now()
           WHERE id = (
               SELECT id FROM jobs
               WHERE status = 'queued' AND run_after <= now()
               ORDER BY created_at
               FOR UPDATE SKIP LOCKED
               LIMIT 1
           )
           RETURNING id, kind, payload as "payload: serde_json::Value""#,
        worker_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ClaimedJob { id: r.id, kind: r.kind, payload: r.payload }))
}

async fn heartbeat(pool: &PgPool, id: Uuid) {
    let _ = sqlx::query!("UPDATE jobs SET locked_at = now() WHERE id = $1", id)
        .execute(pool)
        .await;
}

async fn complete(pool: &PgPool, id: Uuid) {
    let _ = sqlx::query!(
        "UPDATE jobs SET status='succeeded', locked_by=NULL, updated_at=now() WHERE id=$1",
        id
    )
    .execute(pool)
    .await;
}

/// Requeue (or finally fail) a job after a dispatch-level error.
async fn fail(pool: &PgPool, id: Uuid, attempts: i32, max_attempts: i32, err: &str) {
    if attempts < max_attempts {
        let _ = sqlx::query!(
            "UPDATE jobs SET status='queued', locked_by=NULL, last_error=$2,
                    run_after = now() + make_interval(secs => $3), updated_at=now()
             WHERE id=$1",
            id, err, backoff_secs(attempts) as f64
        )
        .execute(pool)
        .await;
    } else {
        let _ = sqlx::query!(
            "UPDATE jobs SET status='failed', locked_by=NULL, last_error=$2, updated_at=now() WHERE id=$1",
            id, err
        )
        .execute(pool)
        .await;
    }
}

/// Requeue jobs stuck in 'running' whose worker stopped heartbeating (crashed /
/// process restarted). Returns the number reclaimed.
pub async fn reclaim_stale(pool: &PgPool) -> u64 {
    sqlx::query!(
        "UPDATE jobs SET status='queued', locked_by=NULL, updated_at=now()
         WHERE status='running' AND locked_at < now() - make_interval(secs => $1)",
        STALE_AFTER.as_secs() as f64
    )
    .execute(pool)
    .await
    .map(|r| r.rows_affected())
    .unwrap_or(0)
}

async fn dispatch(pool: &PgPool, kind: &str, payload: serde_json::Value) -> Result<(), String> {
    match kind {
        "build" => {
            let p: BuildPayload = serde_json::from_value(payload).map_err(|e| e.to_string())?;
            // Strangler switch: opt into the kpack/Buildpacks path with HERMES_BUILDER=kpack;
            // the generated-Dockerfile + kaniko path remains the default.
            if std::env::var("HERMES_BUILDER").as_deref() == Ok("kpack") {
                crate::utils::builder::run_kpack_build(pool.clone(), p.instance_id, p.git_repo, p.branch, p.build_cmd).await;
            } else {
                crate::utils::builder::run_ephemeral_build(pool.clone(), p.instance_id, p.git_repo, p.branch, p.build_cmd).await;
            }
            Ok(())
        }
        "deploy" => {
            let p: DeployPayload = serde_json::from_value(payload).map_err(|e| e.to_string())?;
            let tag = match p.image_tag {
                Some(t) => t,
                None => crate::utils::builder::resolve_instance_image_tag(pool, p.instance_id).await,
            };
            crate::utils::builder::deploy_compiled_app(pool.clone(), p.instance_id, tag).await;
            Ok(())
        }
        other => Err(format!("unknown job kind: {}", other)),
    }
}

/// Start `n` worker loops + a periodic reclaimer. Build/deploy functions own their
/// own success/failure semantics (they set the instance status), so a job is
/// marked succeeded once its handler returns; only dispatch-level errors retry.
pub fn start_workers(pool: PgPool, n: usize) {
    // Immediately reclaim anything orphaned by a previous process.
    let p0 = pool.clone();
    tokio::spawn(async move {
        let reclaimed = reclaim_stale(&p0).await;
        if reclaimed > 0 {
            tracing::warn!(count = reclaimed, "Reclaimed orphaned jobs at startup");
        }
    });

    // Periodic reclaimer.
    let pr = pool.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        loop {
            tick.tick().await;
            reclaim_stale(&pr).await;
        }
    });

    for w in 0..n {
        let pool = pool.clone();
        let worker_id = format!("worker-{}", w);
        tokio::spawn(async move {
            loop {
                match claim_next(&pool, &worker_id).await {
                    Ok(Some(job)) => {
                        // Heartbeat while the handler runs so a long build isn't reclaimed.
                        let hb_pool = pool.clone();
                        let jid = job.id;
                        let hb = tokio::spawn(async move {
                            let mut tick = tokio::time::interval(HEARTBEAT_EVERY);
                            loop {
                                tick.tick().await;
                                heartbeat(&hb_pool, jid).await;
                            }
                        });

                        match dispatch(&pool, &job.kind, job.payload).await {
                            Ok(()) => complete(&pool, job.id).await,
                            Err(e) => {
                                tracing::warn!(job_id = %job.id, kind = %job.kind, "Job dispatch failed: {}", e);
                                // Re-read attempts/max to decide retry vs. give up.
                                if let Ok(rec) = sqlx::query!(
                                    "SELECT attempts, max_attempts FROM jobs WHERE id = $1", job.id
                                ).fetch_one(&pool).await {
                                    fail(&pool, job.id, rec.attempts, rec.max_attempts, &e).await;
                                }
                            }
                        }
                        hb.abort();
                    }
                    Ok(None) => tokio::time::sleep(Duration::from_secs(2)).await,
                    Err(e) => {
                        tracing::warn!("Job claim failed: {}", e);
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::backoff_secs;

    #[test]
    fn backoff_is_monotonic_and_capped() {
        assert_eq!(backoff_secs(0), 10);
        assert_eq!(backoff_secs(1), 20);
        assert_eq!(backoff_secs(2), 40);
        assert_eq!(backoff_secs(3), 80);
        // Capped at 300s, and never panics for large attempt counts.
        assert_eq!(backoff_secs(10), 300);
        assert_eq!(backoff_secs(1000), 300);
        assert!(backoff_secs(0) <= backoff_secs(5));
    }
}
