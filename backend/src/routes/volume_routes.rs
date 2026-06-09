use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::volume_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    let create_vol = Router::new()
        .route("/volumes", post(volume_controller::create_volume))
        .route("/volumes/:volume_id/files/upload", post(volume_controller::upload_volume_file))
        .route("/volumes/:volume_id/files/mkdir", post(volume_controller::create_volume_directory))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    let list_vol = Router::new()
        .route("/apps/:app_id/volumes", get(volume_controller::list_app_volumes))
        .route("/volumes/:volume_id/files", get(volume_controller::list_volume_files))
        .route("/volumes/:volume_id/files/download", get(volume_controller::download_volume_file))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:read")));

    let delete_vol = Router::new()
        .route("/volumes/:volume_id", delete(volume_controller::delete_volume))
        .route("/volumes/:volume_id/files", delete(volume_controller::delete_volume_file))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:delete")));

    Router::new()
        .merge(create_vol)
        .merge(list_vol)
        .merge(delete_vol)
        .with_state(state)
}