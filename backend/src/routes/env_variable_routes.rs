use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn,
};
use crate::app_state::AppState;
use crate::controllers::{env_variable_controller, project_env_controller};
use crate::middlewares::auth_middleware::require_auth;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/envs", post(env_variable_controller::set_env_variable))
        .route("/envs", get(env_variable_controller::list_env_variables))
        .route("/envs/bulk", post(env_variable_controller::set_envs_bulk))
        .route("/envs/:id", delete(env_variable_controller::delete_env_variable))
        .route("/projects/:project_id/envs-grouped", get(env_variable_controller::list_project_envs_grouped))
        // Project-level env pool
        .route("/projects/:project_id/env", get(project_env_controller::list_project_env))
        .route("/projects/:project_id/env", post(project_env_controller::set_project_env))
        .route("/projects/:project_id/env/:id", delete(project_env_controller::delete_project_env))
        // Per-instance opt-in links to the project pool
        .route("/instances/:instance_id/project-env", get(project_env_controller::list_instance_project_env))
        .route("/instances/:instance_id/env-links", post(project_env_controller::link_project_env))
        .route("/instances/:instance_id/env-links/:project_env_id", delete(project_env_controller::unlink_project_env))
        .layer(from_fn(require_auth))
        .with_state(state)
}
