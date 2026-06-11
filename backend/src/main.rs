use std::net::SocketAddr;
use tracing::{info, debug, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

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
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Hermes OS Backend Engine...");

    dotenvy::dotenv().ok();
    let database_url = std::env::var("DATABASE_URL")
        .expect("CRITICAL CONFIG ERROR: DATABASE_URL environment variable is not set.");

    info!("Connecting to PostgreSQL Database...");
    let pool = config::db::init_pool(&database_url).await?;

    crate::utils::cron::start_auto_sleep_worker(pool.clone());
    crate::utils::cron::start_cron_scheduler_engine(pool.clone());
    crate::utils::cron::start_auto_backup_worker(pool.clone());
    crate::utils::health::start_health_check_worker(pool.clone());

    info!("Executing schema migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    let state = app_state::AppState::new(pool);

    info!("Verifying platform core accounts...");
    config::db::seed_initial_super_admin(&state).await?;

    let app = routes::create_router(state)
        .route("/health", axum::routing::get(|| async { "OK" }));

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