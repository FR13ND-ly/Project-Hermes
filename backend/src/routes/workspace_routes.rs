use axum::{
    middleware::from_fn_with_state,
    routing::{get, post, put, delete},
    Router,
    Extension,
};

use crate::app_state::AppState;
use crate::controllers::workspace_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let base_routes = Router::new()
        .route("/workspaces", post(workspace_controller::create_workspace).get(workspace_controller::list_my_workspaces))
        .route("/workspaces/:id", delete(workspace_controller::delete_workspace));

    let developer_routes = Router::new()
        .route("/workspaces/usage", get(workspace_controller::get_workspace_usage))
        .route("/workspaces/current", get(workspace_controller::get_current_workspace))
        .route("/workspaces", put(workspace_controller::update_workspace_settings))
        .route("/workspaces/members", get(workspace_controller::list_workspace_members).post(workspace_controller::add_workspace_member))
        .route("/workspaces/members/:user_id/role", put(workspace_controller::update_workspace_member_role))
        .route("/workspaces/members/:user_id", delete(workspace_controller::remove_workspace_member))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("project:read")));

    let admin_routes = Router::new()
        .route("/admin/workspaces", get(workspace_controller::admin_list_all_workspaces))
        .layer(axum::middleware::from_fn(crate::middlewares::auth_middleware::enforce_super_admin));

    Router::new()
        .merge(base_routes)
        .merge(developer_routes)
        .merge(admin_routes)
        .with_state(state)
}