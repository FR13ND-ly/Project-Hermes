//! Cross-replica coordination backed by Postgres (no Redis), replacing the per-process
//! in-memory primitives so the control plane can run with >1 replica:
//!   * per-workspace advisory locks — serialize a workspace's quota check + write across
//!     replicas so concurrent creates can't overcommit the quota;
//!   * fixed-window rate limiting — shared across replicas.

use sqlx::pool::PoolConnection;
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

use crate::utils::error::AppError;

/// Stable bigint advisory-lock key from a workspace UUID (first 8 bytes).
fn workspace_key(ws_id: Uuid) -> i64 {
    let b = ws_id.as_bytes();
    i64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Held while a workspace's quota check+write runs. On drop, releases the Postgres
/// advisory lock and returns the pinned connection to the pool.
pub struct WorkspaceLock {
    conn: Option<PoolConnection<Postgres>>,
    key: i64,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            let key = self.key;
            tokio::spawn(async move {
                let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
                    .bind(key)
                    .execute(&mut *conn)
                    .await;
            });
        }
    }
}

/// Acquire the per-workspace advisory lock (blocks until granted). Two control-plane
/// replicas mutating the same workspace serialize here, keeping the quota check + insert
/// atomic across replicas. Holds one pool connection for the lock's lifetime, so keep the
/// critical section short.
pub async fn acquire_workspace_lock(pool: &PgPool, ws_id: Uuid) -> Result<WorkspaceLock, AppError> {
    let key = workspace_key(ws_id);
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to acquire lock connection: {}", e)))?;
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(key)
        .execute(&mut *conn)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to take workspace lock: {}", e)))?;
    Ok(WorkspaceLock { conn: Some(conn), key })
}

/// Fixed-window rate limit shared across replicas. Returns `true` if allowed. Increments
/// the counter for `bucket`'s current window, resetting it once the window has elapsed.
/// Fails open on a DB error (don't lock users out on a transient blip).
pub async fn check_rate_limit(pool: &PgPool, bucket: &str, max: i64, window_secs: i64) -> bool {
    let count: Option<i64> = sqlx::query_scalar::<_, i64>(
        "INSERT INTO rate_limit_counters (bucket, window_start, count)
         VALUES ($1, now(), 1)
         ON CONFLICT (bucket) DO UPDATE SET
           count = CASE WHEN rate_limit_counters.window_start < now() - make_interval(secs => $2)
                        THEN 1 ELSE rate_limit_counters.count + 1 END,
           window_start = CASE WHEN rate_limit_counters.window_start < now() - make_interval(secs => $2)
                               THEN now() ELSE rate_limit_counters.window_start END
         RETURNING count::bigint",
    )
    .bind(bucket)
    .bind(window_secs as f64)
    .fetch_one(pool)
    .await
    .ok();

    match count {
        Some(c) => c <= max,
        None => true,
    }
}

/// Global build-concurrency slots. Uses the TWO-int advisory-lock namespace
/// (`pg_*_advisory_lock(classid, objid)`), which is distinct from the bigint namespace
/// the workspace locks use — so build-slot keys never collide with workspace keys.
const BUILD_LOCK_CLASS: i32 = 7777;

/// Holds one global build slot for the duration of a build; releases it (and returns the
/// pinned connection to the pool) on drop.
pub struct BuildSlot {
    conn: Option<PoolConnection<Postgres>>,
    slot: i32,
}

impl Drop for BuildSlot {
    fn drop(&mut self) {
        if let Some(mut conn) = self.conn.take() {
            let slot = self.slot;
            tokio::spawn(async move {
                let _ = sqlx::query("SELECT pg_advisory_unlock($1, $2)")
                    .bind(BUILD_LOCK_CLASS)
                    .bind(slot)
                    .execute(&mut *conn)
                    .await;
            });
        }
    }
}

/// Acquire one of `max` global build slots — a cluster-wide cap on concurrent image
/// builds across ALL replicas (replaces the old per-process semaphore). Blocks (polling
/// every 2s) until a slot frees. The slot is held on a pinned connection for the build's
/// lifetime and released on drop.
pub async fn acquire_build_slot(pool: &PgPool, max: i32) -> BuildSlot {
    let max = max.max(1);
    loop {
        if let Ok(mut conn) = pool.acquire().await {
            for slot in 0..max {
                let got = sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1, $2)")
                    .bind(BUILD_LOCK_CLASS)
                    .bind(slot)
                    .fetch_one(&mut *conn)
                    .await
                    .unwrap_or(false);
                if got {
                    return BuildSlot { conn: Some(conn), slot };
                }
            }
            // No slot free — drop the probe connection and back off before retrying.
            drop(conn);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
