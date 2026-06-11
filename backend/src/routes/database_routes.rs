use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::database_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_router = Router::new()
        .route("/databases", post(database_controller::create_database))
        .route("/databases/:id/settings", post(database_controller::update_database_settings))
        .route("/databases/:id/backups", post(database_controller::create_database_backup))
        .route("/databases/:id/backups/:backup_id/restore", post(database_controller::restore_database_backup))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("db:create")));

    let get_router = Router::new()
        .route("/databases", get(database_controller::list_project_databases))
        .route("/databases/:id", get(database_controller::get_database))
        .route("/databases/:id/reveal", post(database_controller::reveal_database_credentials))
        .route("/databases/:id/query", post(database_controller::execute_database_query))
        .route("/databases/:id/logs", get(database_controller::stream_database_logs))
        .route("/databases/:id/backups", get(database_controller::list_database_backups))
        .route("/databases/:id/metrics", get(database_controller::get_database_metrics))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("db:read")));

    let delete_router = Router::new()
        .route("/databases/:id", delete(database_controller::delete_database))
        .route("/databases/:id/backups/:backup_id", delete(database_controller::delete_database_backup))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("db:delete")));

    Router::new()
        .merge(create_router)
        .merge(get_router)
        .merge(delete_router)
        .with_state(state)
}