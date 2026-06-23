use axum::{
    routing::{post, get, delete, patch},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::cron_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_router = Router::new()
        .route("/cron", post(cron_controller::create_cron_job))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let delete_router = Router::new()
        .route("/cron/:job_id", delete(cron_controller::delete_cron_job))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let update_router = Router::new()
        .route("/cron/:job_id", patch(cron_controller::update_cron_job))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let logs_router = Router::new()
        .route("/cron/:job_id/logs", get(cron_controller::list_cron_job_logs))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let env_router = Router::new()
        .route("/cron/:job_id/env", get(cron_controller::get_cron_env))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let list_router = Router::new()
        .route("/apps/:app_id/cron", get(cron_controller::list_app_cron_jobs))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let project_list_router = Router::new()
        .route("/projects/:project_id/cron", get(cron_controller::list_project_cron_jobs))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    Router::new()
        .merge(create_router)
        .merge(delete_router)
        .merge(update_router)
        .merge(logs_router)
        .merge(env_router)
        .merge(list_router)
        .merge(project_list_router)
        .with_state(state)
}