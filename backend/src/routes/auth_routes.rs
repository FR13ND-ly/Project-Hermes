use axum::{
    middleware::from_fn,
    routing::{post, put, get, delete},
    Router,
};

use crate::app_state::AppState;
use crate::controllers::auth_controller;
use crate::middlewares::auth_middleware::enforce_super_admin;

pub fn routes(state: AppState) -> Router {
    let public_routes = Router::new()
        .route("/login", post(auth_controller::login))
        .route("/activate", post(auth_controller::activate))
        .route("/refresh", post(auth_controller::refresh_session));

    let secure_routes = Router::new()
        .route("/password-change", put(auth_controller::change_password))
        .route("/switch-workspace", post(auth_controller::switch_workspace));

    let super_admin_routes = Router::new()
        .route("/provision-user", post(auth_controller::provision_user))
        .route("/users", get(auth_controller::list_users))
        .route("/users/:id", delete(auth_controller::delete_user))
        .route("/users/:id/reset-password", post(auth_controller::reset_user_password))
        .route("/users/:id/toggle-suspend", post(auth_controller::toggle_user_suspend))
        .route("/system-logs", get(auth_controller::get_system_logs))
        .route("/auth-logs", get(auth_controller::get_auth_logs))
        .route("/gc-runs", get(auth_controller::get_gc_runs))
        .layer(from_fn(enforce_super_admin));

    Router::new()
        .nest("/auth", public_routes)
        .nest("/auth", secure_routes)
        .nest("/users", super_admin_routes)
        .with_state(state)
}