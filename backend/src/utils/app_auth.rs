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
    // Read by key — multiple baas_auth vars (secret/app_id/api_url) now share the
    // same source_id=app_id, so we must pin the secret's key specifically.
    let row = sqlx::query!(
        "SELECT encrypted_value, nonce FROM project_env_variables WHERE source = 'baas_auth' AND source_id = $1 AND key = $2",
        app_id, AUTH_SECRET_ENV_KEY
    )
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        return crypto::decrypt_env_value(&r.encrypted_value, &r.nonce);
    }

    rotate_app_auth_secret(pool, app_id, ws_id, project_id).await
}

/// Upsert a single BaaS integration var into the project env pool under
/// (source='baas_auth', source_id=app_id), keyed by `key`. Unlike `publish_project_env`
/// (one var per source_id), this keys on (project_id, key) so the secret, app_id and
/// api_url can coexist for one app while still being cleaned up together by source_id.
pub async fn publish_baas_var(
    pool: &PgPool,
    ws_id: Uuid,
    project_id: Uuid,
    app_id: Uuid,
    key: &str,
    value: &str,
    is_secret: bool,
) -> Result<(), AppError> {
    let (enc, nonce) = crypto::encrypt_env_value(value)?;
    sqlx::query!(
        "INSERT INTO project_env_variables (id, workspace_id, project_id, key, encrypted_value, nonce, is_secret, source, source_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'baas_auth', $8)
         ON CONFLICT (project_id, key)
         DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, source = 'baas_auth', source_id = $8, updated_at = now()",
        Uuid::new_v4(), ws_id, project_id, key, enc, nonce, is_secret, app_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Force-rotate the app's signing secret: generate a fresh one, persist encrypted,
/// republish to the project pool (upsert on the same key). Returns the new secret.
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
    publish_baas_var(pool, ws_id, project_id, app_id, AUTH_SECRET_ENV_KEY, &secret, true).await?;
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
