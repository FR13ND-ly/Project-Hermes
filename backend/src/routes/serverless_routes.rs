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
    let list_router = Router::new()
        .route("/projects/:project_id/functions", get(serverless_controller::list_functions))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let create_router = Router::new()
        .route("/projects/:project_id/functions", post(serverless_controller::create_function))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let get_router = Router::new()
        .route("/projects/:project_id/functions/:id", get(serverless_controller::get_function))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let update_router = Router::new()
        .route("/projects/:project_id/functions/:id", put(serverless_controller::update_function))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let delete_router = Router::new()
        .route("/projects/:project_id/functions/:id", delete(serverless_controller::delete_function))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let deploy_router = Router::new()
        .route("/projects/:project_id/functions/:id/deploy", post(serverless_controller::deploy_function))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let logs_router = Router::new()
        .route("/projects/:project_id/functions/:id/logs/stream", get(serverless_controller::stream_function_logs))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    Router::new()
        .merge(list_router)
        .merge(create_router)
        .merge(get_router)
        .merge(update_router)
        .merge(delete_router)
        .merge(deploy_router)
        .merge(logs_router)
        .with_state(state)
}
