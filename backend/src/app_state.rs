use sqlx::PgPool;
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub progress_tx: broadcast::Sender<(Uuid, i32)>,
}

impl AppState {
    pub fn new(pool: PgPool) -> Self {
        let (progress_tx, _subscribers_rx) = broadcast::channel(1024);

        Self { 
            pool, 
            progress_tx 
        }
    }
}