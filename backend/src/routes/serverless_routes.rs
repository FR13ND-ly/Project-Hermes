use axum::{
    routing::{post, get, delete, put},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::serverless_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let read_router = Router::new()
        .route("/projects/:project_id/serverless", get(serverless_controller::list_instances))
        .route("/projects/:project_id/serverless/:id", get(serverless_controller::get_instance))
        .route("/projects/:project_id/serverless/:id/routes", get(serverless_controller::list_routes))
        .route("/projects/:project_id/serverless/:id/project-env", get(serverless_controller::list_instance_project_env))
        .route("/projects/:project_id/serverless/:id/env", get(serverless_controller::list_instance_env))
        .route("/projects/:project_id/serverless/:id/metrics", get(serverless_controller::get_instance_metrics))
        .route("/projects/:project_id/serverless/:id/builds", get(serverless_controller::list_instance_builds))
        .route("/projects/:project_id/serverless/:id/builds/:build_id/logs/stream", get(serverless_controller::stream_build_logs))
        .route("/projects/:project_id/serverless/:id/logs/stream", get(serverless_controller::stream_instance_logs))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let deploy_router = Router::new()
        // Instance lifecycle
        .route("/projects/:project_id/serverless", post(serverless_controller::create_instance))
        .route("/projects/:project_id/serverless/:id", put(serverless_controller::update_instance))
        .route("/projects/:project_id/serverless/:id", delete(serverless_controller::delete_instance))
        .route("/projects/:project_id/serverless/:id/deploy", post(serverless_controller::deploy_instance))
        .route("/projects/:project_id/serverless/:id/reload-env", post(serverless_controller::reload_instance_env))
        // Routes
        .route("/projects/:project_id/serverless/:id/routes", post(serverless_controller::create_route))
        .route("/projects/:project_id/serverless/:id/routes/:route_id", put(serverless_controller::update_route))
        .route("/projects/:project_id/serverless/:id/routes/:route_id", delete(serverless_controller::delete_route))
        // Env (instance-level)
        .route("/projects/:project_id/serverless/:id/env", post(serverless_controller::set_instance_env))
        .route("/projects/:project_id/serverless/:id/env/:env_id", delete(serverless_controller::delete_instance_env))
        // Project-pool links
        .route("/projects/:project_id/serverless/:id/env-links", post(serverless_controller::link_instance_project_env))
        .route("/projects/:project_id/serverless/:id/env-links/:project_env_id", delete(serverless_controller::unlink_instance_project_env))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    Router::new()
        .merge(read_router)
        .merge(deploy_router)
        .with_state(state)
}
