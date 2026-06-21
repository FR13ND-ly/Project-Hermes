use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Shared, cheaply-cloneable application state. Cross-replica coordination (workspace
/// quota locks, rate limiting) now lives in Postgres (see `utils::locks`), so the state
/// holds no per-process primitives — keeping the control plane stateless / HA-ready.
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub progress_tx: broadcast::Sender<(Uuid, i32)>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        let (progress_tx, _subscribers_rx) = broadcast::channel(1024);
        Self { pool, progress_tx }
    }
}
