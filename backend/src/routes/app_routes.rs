use axum::{
    routing::{post, get, delete, patch},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::app_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_router = Router::new()
        .route("/apps", post(app_controller::create_app))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:create")));

    let get_router = Router::new()
        .route("/apps/:id", get(app_controller::get_app_details))
        .route("/projects/:project_id/apps", get(app_controller::list_project_apps))
        .route("/apps/:id/instances/:instance_id/logs", get(app_controller::stream_instance_logs))
        .route("/apps/:id/instances/:instance_id/stats", get(app_controller::stream_instance_stats))
        .route("/apps/:id/instances/:instance_id/metrics", get(app_controller::get_instance_metrics))
        .route("/apps/:id/builds", get(app_controller::list_app_builds))
        .route("/apps/:id/builds/:build_id", get(app_controller::get_build_details))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let branch_router = Router::new()
        .route("/apps/:id/branches", post(app_controller::create_branch_instance))
        .route("/apps/:id/instances/:instance_id/settings", patch(app_controller::update_instance_settings))
        .route("/apps/:id/instances/:instance_id/stop", post(app_controller::stop_app_instance))
        .route("/apps/:id/instances/:instance_id/start", post(app_controller::start_app_instance))
        .route("/apps/:id/instances/:instance_id/redeploy", post(app_controller::redeploy_app_instance))
        .route("/apps/:id/instances/:instance_id/serverless", post(app_controller::configure_serverless))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let delete_router = Router::new()
        .route("/apps/:id", delete(app_controller::delete_app))
        .route("/apps/:id/instances/:instance_id", delete(app_controller::delete_app_instance))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:delete")));

    let webhook_router = Router::new()
        .route("/apps/webhook", post(app_controller::handle_github_webhook));

    Router::new()
        .merge(create_router)
        .merge(get_router)
        .merge(branch_router)
        .merge(delete_router)
        .merge(webhook_router)
        .with_state(state)
}