use axum::{
    extract::{ws::{Message, WebSocket, WebSocketUpgrade}, State},
    response::Response,
    routing::get,
    Router,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use std::convert::Infallible;
use tokio::sync::broadcast;
use tracing::{debug, error};

use crate::app_state::AppState;
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::event_broadcaster::SystemEvent;

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    AuthenticatedUser(claims): AuthenticatedUser,
    State(_state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, claims))
}

async fn handle_socket(socket: WebSocket, claims: crate::middlewares::auth_middleware::Claims) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = crate::utils::event_broadcaster::get_ws_sender().subscribe();
    
    let ws_id = claims.current_workspace_id;
    let is_admin = claims.is_super_admin;
    
    // Forward broadcast events to the client (filtered by workspace) while sending a
    // periodic ping so idle connections survive proxy read-timeouts (e.g. nginx 60s).
    let mut send_task = tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = heartbeat.tick() => {
                    if sender.send(Message::Ping(Vec::new())).await.is_err() {
                        break;
                    }
                }
                recv = rx.recv() => match recv {
                    Ok(event) => {
                        // Filter event by workspace_id
                        let belongs_to_workspace = match &event {
                            SystemEvent::InstanceStatusChanged { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::DatabaseStatusChanged { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::BuildStatusChanged { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::IncidentCreated { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::CronJobUpdated { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::CronJobDeleted { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::CronJobLogCreated { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::ServerlessFunctionUpdated { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::ServerlessFunctionDeleted { workspace_id, .. } => Some(*workspace_id) == ws_id,
                            SystemEvent::StorageObjectUpdated { workspace_id, .. } => Some(*workspace_id) == ws_id,
                        };

                        if is_admin || belongs_to_workspace {
                            if let Ok(serialized) = serde_json::to_string(&event) {
                                if let Err(e) = sender.send(Message::Text(serialized)).await {
                                    debug!("Failed to send message over websocket: {}", e);
                                    break;
                                }
                            }
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        // The client fell behind and missed `n` events. Tell it to
                        // refetch so the UI doesn't get stuck on stale state.
                        debug!("WebSocket client lagged, dropped {} events; sending resync", n);
                        let _ = sender.send(Message::Text("{\"type\":\"resync\"}".to_string())).await;
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    });

    // Spawns a task to consume messages from the client (e.g. ping/pong, close)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
            // We ignore other incoming messages for now, just maintaining the connection
        }
    });

    // Wait until either task completes, then abort the other and clean up
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    };
}
