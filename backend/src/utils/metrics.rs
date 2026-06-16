//! Central registry for the platform's own RED/USE metrics (builds, deploys,
//! health, webhooks, cron, DB pool, tenant resource counts), exposed via the
//! Prometheus recorder installed in [`crate::utils::observability`] and scraped
//! at `/metrics`.
//!
//! Call [`describe`] once at startup (so the `# HELP`/`# TYPE` lines are emitted)
//! and use the `record_*` / `inc_*` helpers from the relevant workers so metric
//! names and label keys stay consistent across call sites.

use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram, Unit};

// --- Metric names (single source of truth) ---------------------------------
pub const BUILDS_TOTAL: &str = "hermes_builds_total";
pub const BUILD_DURATION: &str = "hermes_build_duration_seconds";
pub const BUILDS_IN_PROGRESS: &str = "hermes_builds_in_progress";
pub const BUILD_PERMITS_AVAILABLE: &str = "hermes_build_permits_available";
pub const BUILDS_QUEUED: &str = "hermes_builds_queued";
pub const BUILD_FAILURES_TOTAL: &str = "hermes_build_failures_total";

pub const DEPLOYS_TOTAL: &str = "hermes_deploys_total";

pub const HEALTH_CHECKS_TOTAL: &str = "hermes_health_checks_total";
pub const INCIDENTS_OPEN: &str = "hermes_incidents_open";
pub const WEBHOOK_DELIVERIES_TOTAL: &str = "hermes_webhook_deliveries_total";

pub const CRON_RUNS_TOTAL: &str = "hermes_cron_runs_total";
pub const CRON_RUN_DURATION: &str = "hermes_cron_run_duration_seconds";

pub const DB_POOL_CONNECTIONS: &str = "hermes_db_pool_connections";
pub const APPS_RUNNING: &str = "hermes_apps_running";
pub const DATABASES_RUNNING: &str = "hermes_databases_running";
pub const FUNCTIONS_RUNNING: &str = "hermes_functions_running";

/// Register descriptions for every platform metric. Idempotent; call once at boot.
pub fn describe() {
    describe_counter!(BUILDS_TOTAL, "Total image builds, labelled by result (success|failed|cancelled).");
    describe_histogram!(BUILD_DURATION, Unit::Seconds, "Image build wall-clock duration.");
    describe_gauge!(BUILDS_IN_PROGRESS, "Image builds currently executing (have a build permit).");
    describe_gauge!(BUILD_PERMITS_AVAILABLE, "Free build-concurrency permits (0 = saturated).");
    describe_gauge!(BUILDS_QUEUED, "Builds in the 'queued' phase waiting for a permit.");
    describe_counter!(BUILD_FAILURES_TOTAL, "Image build failures, labelled by failure category.");

    describe_counter!(DEPLOYS_TOTAL, "Resource deploys, labelled by kind and result.");

    describe_counter!(HEALTH_CHECKS_TOTAL, "Health probe results (ok|unhealthy|unreachable).");
    describe_gauge!(INCIDENTS_OPEN, "Currently unresolved health incidents.");
    describe_counter!(WEBHOOK_DELIVERIES_TOTAL, "Outbound alert webhook deliveries, by type and result.");

    describe_counter!(CRON_RUNS_TOTAL, "Cron job executions, labelled by result.");
    describe_histogram!(CRON_RUN_DURATION, Unit::Seconds, "Cron job execution duration.");

    describe_gauge!(DB_POOL_CONNECTIONS, "Postgres pool connections, labelled by state (total|idle).");
    describe_gauge!(APPS_RUNNING, "App instances currently in the running state.");
    describe_gauge!(DATABASES_RUNNING, "Databases currently in the running state.");
    describe_gauge!(FUNCTIONS_RUNNING, "Serverless functions currently active.");
}

// --- Emit helpers ----------------------------------------------------------

/// A build finished. `result` is one of `success` / `failed` / `cancelled`.
pub fn record_build_finished(result: &str, duration_secs: f64) {
    counter!(BUILDS_TOTAL, "result" => result.to_string()).increment(1);
    histogram!(BUILD_DURATION).record(duration_secs);
}

/// A build failed; record its failure category (MANIFEST, POD_CREATE, …).
pub fn record_build_failure_category(category: &str) {
    counter!(BUILD_FAILURES_TOTAL, "category" => category.to_string()).increment(1);
}

/// RAII guard for the `hermes_builds_in_progress` gauge: increments on creation,
/// decrements on drop. Hold one for the lifetime of a build (after the build
/// permit is acquired) so the gauge stays correct across every early return.
pub struct BuildInProgressGuard;

impl BuildInProgressGuard {
    pub fn new() -> Self {
        gauge!(BUILDS_IN_PROGRESS).increment(1.0);
        Self
    }
}

impl Drop for BuildInProgressGuard {
    fn drop(&mut self) {
        gauge!(BUILDS_IN_PROGRESS).decrement(1.0);
    }
}

/// A resource deploy was attempted. `kind` e.g. `app`/`serverless`/`database`,
/// `result` `success`/`failed`.
pub fn record_deploy(kind: &str, result: &str) {
    counter!(DEPLOYS_TOTAL, "kind" => kind.to_string(), "result" => result.to_string()).increment(1);
}

/// A health probe completed. `result` is `ok` / `unhealthy` / `unreachable`.
pub fn record_health_check(result: &str) {
    counter!(HEALTH_CHECKS_TOTAL, "result" => result.to_string()).increment(1);
}

/// An alert webhook delivery was attempted. `result` is `success` / `failed`.
pub fn record_webhook(webhook_type: &str, result: &str) {
    counter!(WEBHOOK_DELIVERIES_TOTAL, "type" => webhook_type.to_string(), "result" => result.to_string())
        .increment(1);
}

/// A cron job execution finished. `result` is `success` / `failed`.
pub fn record_cron_run(result: &str, duration_secs: f64) {
    counter!(CRON_RUNS_TOTAL, "result" => result.to_string()).increment(1);
    histogram!(CRON_RUN_DURATION).record(duration_secs);
}

// --- Gauge sampler ---------------------------------------------------------

/// Periodically sample point-in-time gauges that can't be event-driven: DB pool
/// utilization, build-concurrency saturation, queued builds, running resource
/// counts and open incidents. Spawned once at startup from `main`.
pub fn start_gauge_sampler(pool: sqlx::PgPool) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            interval.tick().await;

            // Connection pool state.
            gauge!(DB_POOL_CONNECTIONS, "state" => "total").set(pool.size() as f64);
            gauge!(DB_POOL_CONNECTIONS, "state" => "idle").set(pool.num_idle() as f64);

            // Build concurrency headroom (0 = saturated).
            gauge!(BUILD_PERMITS_AVAILABLE)
                .set(crate::utils::builder::available_build_permits() as f64);

            // Counts (best-effort; a failed query just skips this tick's update).
            if let Ok(n) = sqlx::query_scalar!("SELECT count(*) FROM app_builds WHERE phase = 'queued'")
                .fetch_one(&pool).await
            {
                gauge!(BUILDS_QUEUED).set(n.unwrap_or(0) as f64);
            }
            if let Ok(n) = sqlx::query_scalar!("SELECT count(*) FROM app_instances WHERE status = 'running'")
                .fetch_one(&pool).await
            {
                gauge!(APPS_RUNNING).set(n.unwrap_or(0) as f64);
            }
            if let Ok(n) = sqlx::query_scalar!("SELECT count(*) FROM databases WHERE status = 'running'")
                .fetch_one(&pool).await
            {
                gauge!(DATABASES_RUNNING).set(n.unwrap_or(0) as f64);
            }
            if let Ok(n) = sqlx::query_scalar!("SELECT count(*) FROM serverless_instances WHERE status = 'active'")
                .fetch_one(&pool).await
            {
                gauge!(FUNCTIONS_RUNNING).set(n.unwrap_or(0) as f64);
            }
            if let Ok(n) = sqlx::query_scalar!("SELECT count(*) FROM app_incident_logs WHERE resolved_at IS NULL")
                .fetch_one(&pool).await
            {
                gauge!(INCIDENTS_OPEN).set(n.unwrap_or(0) as f64);
            }
        }
    });
}
