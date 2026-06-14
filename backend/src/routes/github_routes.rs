use axum::{
    routing::{get, post},
    Router,
};

use crate::app_state::AppState;
use crate::controllers::github_controller;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/github/token", post(github_controller::link_github_token))
        .route("/github/repos", get(github_controller::list_github_repos))
        .route("/github/repos/:owner/:repo/branches", get(github_controller::list_github_branches))
        .route("/github/repos/:owner/:repo/detect", get(github_controller::detect_project_type))
        .route("/github/repos/:owner/:repo/compose", get(github_controller::get_repo_compose))
        .with_state(state)
}
