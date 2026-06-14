use axum::{
    routing::post,
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::compose_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/stacks/import", post(compose_controller::import_compose_stack))
        .route("/stacks/plan", post(compose_controller::plan_compose))
        .route("/stacks/apply", post(compose_controller::apply_compose_plan))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:create")))
        .with_state(state)
}