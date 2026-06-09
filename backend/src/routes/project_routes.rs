use axum::{
    middleware::from_fn_with_state,
    routing::{get, post, delete},
    Router,
    Extension,
};

use crate::app_state::AppState;
use crate::controllers::project_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let post_router = Router::new()
        .route("/projects", post(project_controller::create_project))
        .route("/projects/:id/ssh-keys", post(project_controller::create_project_ssh_key))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:create")));

    let get_router = Router::new()
        .route("/projects", get(project_controller::list_workspace_projects))
        .route("/projects/:id", get(project_controller::get_project))
        .route("/projects/:id/ssh-keys", get(project_controller::list_project_ssh_keys))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:read")));

    let delete_router = Router::new()
        .route("/projects/:id", delete(project_controller::delete_project))
        .route("/projects/:id/ssh-keys/:key_id", delete(project_controller::delete_project_ssh_key))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:delete")));

    Router::new()
        .merge(post_router)
        .merge(get_router)
        .merge(delete_router)
        .with_state(state)
}