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
    let database_url = std::env::var("DATABASE_URL")
        .expect("CRITICAL CONFIG ERROR: DATABASE_URL environment variable is not set.");

    info!("Connecting to PostgreSQL Database...");
    let pool = config::db::init_pool(&database_url).await?;

    crate::utils::cron::start_auto_sleep_worker(pool.clone());
    crate::utils::cron::start_cron_scheduler_engine(pool.clone());
    crate::utils::health::start_health_check_worker(pool.clone());
    crate::utils::health::start_metric_alert_worker(pool.clone());
    crate::utils::metrics::start_gauge_sampler(pool.clone());

    info!("Executing schema migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    // Auto-backups are now real, editable cron jobs; ensure existing backup-enabled
    // databases have one (the dedicated backup worker has been retired).
    crate::utils::cron::reconcile_backup_crons(&pool).await;

    // Migrate legacy per-user GitHub tokens into workspace git credentials.
    crate::utils::git_provider::reconcile_git_credentials(&pool).await;

    // Backfill per-bucket access credentials (app_id/secret_key) + publish them.
    crate::controllers::storage_controller::reconcile_bucket_credentials(&pool).await;

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
        
    axum::serve(listener, app).await
        .expect("Failed to start Axum engine server.");

    Ok(())
}

/// Prometheus exposition endpoint for the backend's own RED metrics.
async fn metrics_endpoint() -> axum::response::Response {
    use axum::response::IntoResponse;
    match crate::utils::observability::metrics_handle() {
        Some(h) => h.render().into_response(),
        None => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "metrics recorder not installed",
        )
            .into_response(),
    }
}