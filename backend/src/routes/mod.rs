use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use crate::app_state::AppState;

mod auth_routes;
mod domain_routes;
mod project_routes;
mod storage_routes;
mod workspace_routes;
mod env_variable_routes;
mod database_routes;
mod app_routes;
mod app_user_routes;
mod volume_routes;
mod compose_routes; // <-- ADAUGĂ ACEASTĂ LINIE
mod cron_routes;
mod github_routes;
mod incident_routes;
mod webhook_routes;
mod ws_routes;
mod serverless_routes;

pub fn create_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route(
            "/storage/assets/:workspace_id/:bucket_slug/*file_path",
            axum::routing::get(crate::controllers::storage_controller::serve_public_file).with_state(state.clone()),
        )
        .nest("/api/v1", auth_routes::routes(state.clone()))
        .nest("/api/v1", domain_routes::routes(state.clone()))
        .nest("/api/v1", project_routes::routes(state.clone()))
        .nest("/api/v1", workspace_routes::routes(state.clone()))
        .nest("/api/v1/storage", storage_routes::routes(state.clone()))
        .nest("/api/v1", env_variable_routes::routes(state.clone()))
        .nest("/api/v1", database_routes::routes(state.clone()))
        .nest("/api/v1", app_routes::routes(state.clone()))
        .nest("/api/v1", app_user_routes::routes(state.clone()))
        .nest("/api/v1", volume_routes::routes(state.clone()))
        .nest("/api/v1", compose_routes::routes(state.clone()))
        .nest("/api/v1", cron_routes::routes(state.clone()))
        .nest("/api/v1", incident_routes::routes(state.clone()))
        .nest("/api/v1", webhook_routes::routes(state.clone()))
        .nest("/api/v1", github_routes::routes(state.clone()))
        .nest("/api/v1", serverless_routes::routes(state.clone()))
        .nest("/api/v1", ws_routes::routes(state))
        .layer(axum::middleware::from_fn(crate::middlewares::logger::telemetry_logger))
        .layer(cors)
}