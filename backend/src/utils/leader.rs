//! Leader election for the control plane, so it can run with >1 replica without the
//! singleton background workers (cron, auto-sleep, health, reconcile, …) double-running.
//!
//! Mechanism: a single-row lease in Postgres (`leader_lease`). Each replica has a random
//! `node_id`; every `RENEW_EVERY` it tries to take/renew the lease with an atomic UPSERT
//! that only succeeds if the lease is unheld, expired, or already ours. The winner sets a
//! process-global `IS_LEADER` flag the workers check each tick. If the leader dies, the
//! lease expires after `LEASE_TTL` and another replica takes over (brief leaderless gap,
//! which is fine for these periodic workers). No new infrastructure — Postgres only.
//!
//! With a single replica this is always the leader, so behaviour is unchanged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

static IS_LEADER: AtomicBool = AtomicBool::new(false);

/// How long a renewed lease stays valid. Must be comfortably larger than `RENEW_EVERY`
/// so a momentarily slow renew doesn't drop leadership.
const LEASE_TTL_SECS: f64 = 30.0;
const RENEW_EVERY: Duration = Duration::from_secs(10);

/// True if this replica currently holds the leader lease. Singleton workers gate their
/// per-tick work on this.
pub fn is_leader() -> bool {
    IS_LEADER.load(Ordering::Relaxed)
}

/// Spawn the lease renew/acquire loop. Call once at startup.
pub fn start_leader_elector(pool: PgPool) {
    let node_id = Uuid::new_v4();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(RENEW_EVERY);
        loop {
            interval.tick().await;
            // Take or renew the lease: the conditional UPDATE only fires when the lease is
            // ours or expired, so a healthy other holder keeps it (no row returned → not us).
            let won = sqlx::query_scalar::<_, bool>(
                "INSERT INTO leader_lease (id, holder, expires_at)
                 VALUES (1, $1, now() + make_interval(secs => $2))
                 ON CONFLICT (id) DO UPDATE
                   SET holder = EXCLUDED.holder, expires_at = EXCLUDED.expires_at
                   WHERE leader_lease.holder = $1 OR leader_lease.expires_at < now()
                 RETURNING holder = $1",
            )
            .bind(node_id)
            .bind(LEASE_TTL_SECS)
            .fetch_optional(&pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(false);

            let was = IS_LEADER.swap(won, Ordering::Relaxed);
            if won && !was {
                tracing::info!(%node_id, "Acquired control-plane leadership");
            } else if !won && was {
                tracing::warn!(%node_id, "Lost control-plane leadership");
            }
        }
    });
}
