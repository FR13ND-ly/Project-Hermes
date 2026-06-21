use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use crate::app_state::AppState;

/// Build the CORS layer. In production set `CORS_ALLOWED_ORIGINS` to a comma
/// separated allowlist (e.g. `https://app.example.com,https://admin.example.com`).
/// When unset or `*`, all origins are allowed — convenient for local development.
fn build_cors() -> CorsLayer {
    match std::env::var("CORS_ALLOWED_ORIGINS") {
        Ok(val) if !val.trim().is_empty() && val.trim() != "*" => {
            let origins: Vec<_> = val
                .split(',')
                .filter_map(|o| o.trim().parse::<axum::http::HeaderValue>().ok())
                .collect();
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(origins))
                .allow_methods(Any)
                .allow_headers(Any)
        }
        _ => CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    }
}

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
mod git_routes;
mod incident_routes;
mod webhook_routes;
mod ws_routes;
mod serverless_routes;

pub fn create_router(state: AppState) -> Router {
    let cors = build_cors();

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
        .nest("/api/v1", git_routes::routes(state.clone()))
        .nest("/api/v1", serverless_routes::routes(state.clone()))
        .nest("/api/v1", ws_routes::routes(state))
        .layer(axum::middleware::from_fn(crate::middlewares::logger::telemetry_logger))
        .layer(cors)
}