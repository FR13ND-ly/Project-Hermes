use axum::{
    routing::{get, delete},
    Router,
};

use crate::app_state::AppState;
use crate::controllers::cloudflare_controller;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/cloudflare-credentials",
            get(cloudflare_controller::list_cloudflare_credentials)
                .post(cloudflare_controller::create_cloudflare_credential),
        )
        .route(
            "/cloudflare-credentials/:id",
            delete(cloudflare_controller::delete_cloudflare_credential),
        )
        .with_state(state)
}
