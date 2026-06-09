use axum::{
    routing::{get, post},
    Router,
};

use crate::app_state::AppState;
use crate::controllers::incident_controller;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/projects/:project_id/incidents", get(incident_controller::list_project_incidents))
        .route("/incidents/:incident_id/resolve", post(incident_controller::resolve_incident))
        .with_state(state)
}
