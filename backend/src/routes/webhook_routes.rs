use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::webhook_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_routes = Router::new()
        .route("/projects/:project_id/webhooks", post(webhook_controller::create_webhook))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:create")));

    let read_routes = Router::new()
        .route("/projects/:project_id/webhooks", get(webhook_controller::list_project_webhooks))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:read")));

    let delete_routes = Router::new()
        .route("/projects/:project_id/webhooks/:webhook_id", delete(webhook_controller::delete_webhook))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:delete")));

    Router::new()
        .merge(create_routes)
        .merge(read_routes)
        .merge(delete_routes)
        .with_state(state)
}
