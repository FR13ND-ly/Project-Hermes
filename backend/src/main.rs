use std::net::SocketAddr;
use tracing::info;

mod app_state;
mod config;
mod controllers;
mod dtos;
mod middlewares;
mod models;
mod routes;
mod utils;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    crate::utils::observability::init();

    info!("Starting Hermes OS Backend Engine...");

    dotenvy::dotenv().ok();

    // Fail fast if any mandatory secret is missing/invalid, before binding a port.
    config::secrets::validate()?;

    let database_url = std::env::var("DATABASE_URL")
        .expect("CRITICAL CONFIG ERROR: DATABASE_URL environment variable is not set.");

    info!("Connecting to PostgreSQL Database...");
    let pool = config::db::init_pool(&database_url).await?;

    crate::utils::cron::start_auto_sleep_worker(pool.clone());
    crate::utils::cron::start_cron_scheduler_engine(pool.clone());
    crate::utils::health::start_health_check_worker(pool.clone());
    crate::utils::health::start_metric_alert_worker(pool.clone());
    crate::utils::metrics::start_gauge_sampler(pool.clone());
    crate::utils::builder::start_stuck_deploy_reconciler(pool.clone());
    // Steady-state reconcile loop (self-heals drifted/deleted Deployments). Now on by
    // default; set HERMES_RECONCILE=off to disable it (the strangler escape hatch).
    if std::env::var("HERMES_RECONCILE").as_deref() != Ok("off") {
        crate::utils::builder::start_reconcile_worker(pool.clone());
    }
    // Durable build/deploy job workers (survive restarts; replace fire-and-forget).
    crate::utils::job_queue::start_workers(pool.clone(), 2);

    info!("Executing schema migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    // Leader election: only the elected replica runs the singleton workers above
    // (they gate each tick on `leader::is_leader`). With one replica it's always the
    // leader, so behaviour is unchanged. Started after migrations so the lease table exists.
    crate::utils::leader::start_leader_elector(pool.clone());

    // Chaos simulator: leader-gated worker that auto-reverts experiments whose window
    // has elapsed (and reclaims any left running across a restart). Started after
    // migrations so the chaos_experiments table exists.
    crate::utils::chaos::start_chaos_worker(pool.clone());

    // Auto-backups are now real, editable cron jobs; ensure existing backup-enabled
    // databases have one (the dedicated backup worker has been retired).
    crate::utils::cron::reconcile_backup_crons(&pool).await;

    // Migrate legacy per-user GitHub tokens into workspace git credentials.
    crate::utils::git_provider::reconcile_git_credentials(&pool).await;

    // Backfill per-bucket access credentials (app_id/secret_key) + publish them.
    crate::controllers::storage_controller::reconcile_bucket_credentials(&pool).await;

    // Republish each BaaS service's HERMES_AUTH_APP_ID/HERMES_APP_ID (= service id) so
    // the values match the new /baas/:id routes after the app→service migration.
    crate::utils::app_auth::reconcile_baas_published_ids(&pool).await;


    // Unstick app instances whose deploy/build monitoring died with a previous
    // process (they'd otherwise show "deploying" forever in the build queue).
    crate::utils::builder::reconcile_stuck_deploys(&pool).await;

    let state = app_state::AppState::new(pool);

    info!("Verifying platform core accounts...");
    config::db::seed_initial_super_admin(&state).await?;

    let app = routes::create_router(state)
        .route("/health", axum::routing::get(|| async { "OK" }))
        .route("/metrics", axum::routing::get(metrics_endpoint));

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr: SocketAddr = format!("0.0.0.0:{}", port)
        .parse()
        .expect("Invalid server port or address format.");

    info!("Hermes API Engine successfully bound to http://{}", addr);
    
    let listener = tokio::net::TcpListener::bind(addr).await
        .expect("Failed to bind TcpListener to address.");
        
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Failed to start Axum engine server.");

    Ok(())
}

/// Resolve when the process is asked to stop (SIGTERM from Kubernetes on a rolling
/// update / scale-down, or Ctrl-C in dev), so in-flight requests drain instead of being
/// dropped — a prerequisite for zero-downtime rollouts.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    info!("Shutdown signal received — draining in-flight requests...");
}

/// Prometheus exposition endpoint for the backend's own RED metrics.
///
/// When `METRICS_TOKEN` is set the endpoint requires `Authorization: Bearer <token>`
/// so the metrics aren't world-readable. If the variable is unset the endpoint
/// stays open (convenient for local/dev scraping).
async fn metrics_endpoint(headers: axum::http::HeaderMap) -> axum::response::Response {
    use axum::response::IntoResponse;

    if let Ok(expected) = std::env::var("METRICS_TOKEN") {
        let provided = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));
        if provided != Some(expected.as_str()) {
            return (axum::http::StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        }
    }

    match crate::utils::observability::metrics_handle() {
        Some(h) => h.render().into_response(),
        None => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder not installed",
        )
            .into_response(),
    }
}