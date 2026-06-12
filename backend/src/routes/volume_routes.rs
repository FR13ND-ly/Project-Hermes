use axum::{
    routing::{get, post, delete},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::volume_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    // PVCs themselves are created only at build time (Dockerfile VOLUME); there is
    // no create-volume endpoint. Files inside a PVC can be browsed and managed.
    let write_files = Router::new()
        .route("/volumes/:volume_id/files/upload", post(volume_controller::upload_volume_file))
        .route("/volumes/:volume_id/files/mkdir", post(volume_controller::create_volume_directory))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:create")));

    let read_vol = Router::new()
        .route("/projects/:project_id/volumes", get(volume_controller::list_project_volumes))
        .route("/volumes/:volume_id/files", get(volume_controller::list_volume_files))
        .route("/volumes/:volume_id/files/download", get(volume_controller::download_volume_file))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:read")));

    let delete_files = Router::new()
        .route("/volumes/:volume_id/files", delete(volume_controller::delete_volume_file))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("volume:delete")));

    Router::new()
        .merge(write_files)
        .merge(read_vol)
        .merge(delete_files)
        .with_state(state)
}
