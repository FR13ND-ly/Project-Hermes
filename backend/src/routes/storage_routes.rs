use axum::{
    routing::{post, get},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::storage_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_bucket_router = Router::new()
        .route("/buckets", post(storage_controller::create_bucket))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    let list_buckets_router = Router::new()
        .route("/buckets", get(storage_controller::list_buckets))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:read")));

    let delete_bucket_router = Router::new()
        .route("/buckets/:id", axum::routing::delete(storage_controller::delete_bucket))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:delete")));

    let update_bucket_router = Router::new()
        .route("/buckets/:id", axum::routing::patch(storage_controller::update_bucket))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    let init_upload_router = Router::new()
        .route("/upload/init", post(storage_controller::initialize_upload))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    let list_objects_router = Router::new()
        .route("/buckets/:bucket_slug/objects", get(storage_controller::list_objects))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:read")));

    let delete_object_router = Router::new()
        .route("/objects/:id", axum::routing::delete(storage_controller::delete_object))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:delete")));

    let download_private_router = Router::new()
        .route("/private/:file_id", get(storage_controller::download_private_file))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:read")));

    let public_transfer_routes = Router::new()
        .route("/upload/:id", post(storage_controller::process_upload_stream).put(storage_controller::process_upload_stream))
        .route("/upload/:id/progress", get(storage_controller::upload_progress_stream));

    let generate_token_router = Router::new()
        .route("/buckets/:id/token", post(storage_controller::generate_bucket_token))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    Router::new()
        .merge(create_bucket_router)
        .merge(list_buckets_router)
        .merge(delete_bucket_router)
        .merge(update_bucket_router)
        .merge(init_upload_router)
        .merge(list_objects_router)
        .merge(delete_object_router)
        .merge(download_private_router)
        .merge(generate_token_router)
        .merge(public_transfer_routes)
        .with_state(state)
}