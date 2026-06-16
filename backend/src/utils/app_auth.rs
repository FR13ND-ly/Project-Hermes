//! BaaS end-user auth helpers (Model 1: per-app HS256 secret + local verify).
//!
//! Each app owns a signing secret stored encrypted on `apps`. Hermes signs
//! end-user JWTs with it and publishes it into the project env pool as
//! `HERMES_AUTH_SECRET`, so the deployed app can verify tokens locally (no
//! round-trip to Hermes). The secret is generated lazily on first need.

use sqlx::PgPool;
use uuid::Uuid;

use crate::utils::crypto;
use crate::utils::error::AppError;

/// The env key under which an app's signing secret is published to its project pool.
pub const AUTH_SECRET_ENV_KEY: &str = "HERMES_AUTH_SECRET";

/// Generate a 48-char alphanumeric signing secret.
fn generate_secret() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    (0..48)
        .map(|_| {
            let idx = (rand::random::<u32>() as usize) % CHARS.len();
            CHARS[idx] as char
        })
        .collect()
}

/// Return the app's signing secret, generating + persisting + publishing it on
/// first call. Idempotent: subsequent calls decrypt and return the stored value.
pub async fn get_or_create_app_auth_secret(
    pool: &PgPool,
    app_id: Uuid,
    ws_id: Uuid,
    project_id: Uuid,
) -> Result<String, AppError> {
    let row = sqlx::query!(
        "SELECT auth_secret, auth_secret_nonce FROM apps WHERE id = $1",
        app_id
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Application not found.".to_string()))?;

    if let (Some(enc), Some(nonce)) = (row.auth_secret, row.auth_secret_nonce) {
        return crypto::decrypt_env_value(&enc, &nonce);
    }

    // First use: generate + publish (same path as an explicit rotation).
    rotate_app_auth_secret(pool, app_id, ws_id, project_id).await
}

/// Force-rotate the app's signing secret: generate a fresh one, persist encrypted,
/// republish to the project pool (upsert on the same key) and ensure the app's
/// instances are linked so it lands in their env. Returns the new secret.
///
/// Side effect: every end-user JWT signed with the previous secret stops verifying,
/// so existing end-user sessions must re-authenticate. Apps pick up the new
/// `HERMES_AUTH_SECRET` on their next reload (caller is responsible for that).
pub async fn rotate_app_auth_secret(
    pool: &PgPool,
    app_id: Uuid,
    ws_id: Uuid,
    project_id: Uuid,
) -> Result<String, AppError> {
    let secret = generate_secret();
    let (enc, nonce) = crypto::encrypt_env_value(&secret)?;
    sqlx::query!(
        "UPDATE apps SET auth_secret = $1, auth_secret_nonce = $2, updated_at = now() WHERE id = $3",
        enc,
        nonce,
        app_id
    )
    .execute(pool)
    .await?;

    let project_env_id = crate::utils::app_env::publish_project_env(
        pool,
        ws_id,
        project_id,
        AUTH_SECRET_ENV_KEY,
        &secret,
        true,
        "baas_auth",
        app_id,
    )
    .await?;

    if let Ok(instances) =
        sqlx::query_scalar!("SELECT id FROM app_instances WHERE app_id = $1", app_id)
            .fetch_all(pool)
            .await
    {
        for inst in instances {
            let _ = sqlx::query!(
                "INSERT INTO app_env_links (app_instance_id, project_env_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
                inst,
                project_env_id
            )
            .execute(pool)
            .await;
        }
    }

    Ok(secret)
}

/// Union of permissions granted to a set of roles, per the app's `auth_roles_config`
/// JSON ({ "role": ["perm", ...] }). Deduplicated and sorted for determinism.
pub fn permissions_for_roles(
    auth_roles_config: &serde_json::Value,
    roles: &[String],
) -> Vec<String> {
    let mut perms: Vec<String> = Vec::new();
    if let Some(map) = auth_roles_config.as_object() {
        for role in roles {
            if let Some(list) = map.get(role).and_then(|v| v.as_array()) {
                for p in list {
                    if let Some(s) = p.as_str() {
                        if !perms.iter().any(|e| e == s) {
                            perms.push(s.to_string());
                        }
                    }
                }
            }
        }
    }
    perms.sort();
    perms
}
