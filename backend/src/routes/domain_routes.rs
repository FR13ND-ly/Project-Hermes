use axum::{
    middleware::from_fn_with_state,
    routing::{post, get, delete, put},
    Router,
    Extension,
};

use crate::app_state::AppState;
use crate::controllers::domain_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let add_router = Router::new()
        .route("/domains", post(domain_controller::add_domain))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("domain:create")));

    let list_router = Router::new()
        .route("/domains", get(domain_controller::list_domains))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("domain:read")));

    let verify_router = Router::new()
        .route("/domains/:id/verify", post(domain_controller::verify_and_sync_domain))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("domain:create")));

    let remove_router = Router::new()
        .route("/domains/:id", delete(domain_controller::remove_domain))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("domain:delete")));

    let update_router = Router::new()
        .route("/domains/:id", put(domain_controller::update_domain))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("domain:create")));

    Router::new()
        .merge(add_router)
        .merge(list_router)
        .merge(verify_router)
        .merge(remove_router)
        .merge(update_router)
        .with_state(state)
}