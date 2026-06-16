use axum::{
    routing::{get, post, delete},
    Router,
};

use crate::app_state::AppState;
use crate::controllers::git_controller;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/git/credentials", get(git_controller::list_credentials).post(git_controller::create_credential))
        .route("/git/credentials/:id", delete(git_controller::delete_credential))
        .route("/git/credentials/:id/repos", get(git_controller::list_repos))
        .route("/git/credentials/:id/branches", get(git_controller::list_branches))
        .route("/git/credentials/:id/detect", get(git_controller::detect_project))
        .route("/git/credentials/:id/compose", get(git_controller::get_compose))
        .with_state(state)
}
