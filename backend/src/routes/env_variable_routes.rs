use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn,
};
use crate::app_state::AppState;
use crate::controllers::env_variable_controller;
use crate::middlewares::auth_middleware::require_auth;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/envs", post(env_variable_controller::set_env_variable))
        .route("/envs", get(env_variable_controller::list_env_variables))
        .route("/envs/:id", delete(env_variable_controller::delete_env_variable))
        .layer(from_fn(require_auth))
        .with_state(state)
}