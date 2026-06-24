use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::app_state::AppState;
use crate::utils::crypto;

pub async fn init_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(50)
        .acquire_timeout(std::time::Duration::from_secs(3))
        .connect(database_url)
        .await
}

pub async fn seed_initial_super_admin(state: &AppState) -> Result<(), anyhow::Error> {
    let super_admin_exists = sqlx::query_scalar!(
        "SELECT EXISTS (SELECT 1 FROM users WHERE is_super_admin = TRUE)"
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if super_admin_exists {
        return Ok(());
    }

    warn!("No Super Admin found. Bootstrapping initial platform root account...");

    let root_email = std::env::var("HERMES_ROOT_EMAIL")
        .unwrap_or_else(|_| "admin@hermes.platform".to_string());
    let root_username = std::env::var("HERMES_ROOT_USERNAME")
        .unwrap_or_else(|_| "root".to_string());
    let root_raw_password = std::env::var("HERMES_ROOT_PASSWORD")
        .expect("CRITICAL: HERMES_ROOT_PASSWORD must be set for first-time platform boot.");

    let max_memory: i32 = std::env::var("WORKSPACE_MAX_MEMORY_MB")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    let max_storage: i32 = std::env::var("WORKSPACE_MAX_STORAGE_GB")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap_or(0);

    let hashed_password = crypto::hash_password(&root_raw_password)
        .map_err(|e| anyhow::anyhow!("Crypto error: {:?}", e))?;
        
    let user_id = Uuid::new_v4();
    let mut tx = state.pool.begin().await?;

    sqlx::query!(
        "INSERT INTO users (id, username, email, password_hash, is_super_admin, status) VALUES ($1, $2, $3, $4, TRUE, 'active'::user_status)",
        user_id, root_username, root_email, hashed_password
    )
    .execute(&mut *tx)
    .await?;

    let workspace_id = Uuid::new_v4();
    sqlx::query!(
        "INSERT INTO workspaces (id, name, slug, created_by, max_memory_mb, max_storage_gb) VALUES ($1, 'Hermes System', 'hermes-system', $2, $3, $4)",
        workspace_id, user_id, max_memory, max_storage
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query!(
        "INSERT INTO workspace_members (workspace_id, user_id, role_id) VALUES ($1, $2, (SELECT id FROM roles WHERE name = 'owner'))",
        workspace_id, user_id
    )
    .execute(&mut *tx)
    .await?;

    // 6. Set the root user's current workspace so it aligns with the middleware
    sqlx::query!("UPDATE users SET current_workspace_id = $1 WHERE id = $2", workspace_id, user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    info!("Root Super Admin successfully seeded. Platform is operational.");

    Ok(())
}