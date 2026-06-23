//! BaaS end-user auth helpers (Model 1: per-service HS256 secret + local verify).
//!
//! Each **BaaS service** owns a signing secret. Hermes signs end-user JWTs with it
//! and publishes it into the project env pool as `HERMES_AUTH_SECRET` (source='baas_auth',
//! source_id = the service id), so a linked app verifies tokens locally with no
//! round-trip to Hermes. The secret is generated lazily on first need.

use sqlx::PgPool;
use uuid::Uuid;

use crate::utils::crypto;
use crate::utils::error::AppError;

/// The env key under which a service's signing secret is published to its project pool.
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

/// Return the service's signing secret, generating + persisting + publishing it on
/// first call. Idempotent: subsequent calls decrypt and return the stored value.
pub async fn get_or_create_secret(
    pool: &PgPool,
    baas_id: Uuid,
    ws_id: Uuid,
    project_id: Uuid,
) -> Result<String, AppError> {
    // Read by key — multiple baas_auth vars (secret/app_id/api_url) share the same
    // source_id = service id, so we must pin the secret's key specifically.
    let row = sqlx::query!(
        "SELECT encrypted_value, nonce FROM project_env_variables WHERE source = 'baas_auth' AND source_id = $1 AND key = $2",
        baas_id, AUTH_SECRET_ENV_KEY
    )
    .fetch_optional(pool)
    .await?;

    if let Some(r) = row {
        return crypto::decrypt_env_value(&r.encrypted_value, &r.nonce);
    }

    rotate_secret(pool, baas_id, ws_id, project_id).await
}

/// Upsert a single BaaS integration var into the project env pool under
/// (source='baas_auth', source_id=app_id), keyed by `key`. Unlike `publish_project_env`
/// (one var per source_id), this keys on (project_id, key) so the secret, app_id and
/// api_url can coexist for one app while still being cleaned up together by source_id.
pub async fn publish_baas_var(
    pool: &PgPool,
    ws_id: Uuid,
    project_id: Uuid,
    baas_id: Uuid,
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
        Uuid::new_v4(), ws_id, project_id, key, enc, nonce, is_secret, baas_id
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
pub async fn rotate_secret(
    pool: &PgPool,
    baas_id: Uuid,
    ws_id: Uuid,
    project_id: Uuid,
) -> Result<String, AppError> {
    let secret = generate_secret();
    publish_baas_var(pool, ws_id, project_id, baas_id, AUTH_SECRET_ENV_KEY, &secret, true).await?;
    Ok(secret)
}

/// Slugify a service name into a project-unique-ish fragment (lowercase, hyphenated).
fn slugify(name: &str) -> String {
    let s: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "auth".to_string() } else { s }
}

/// Create a standalone BaaS service row in a project + generate & publish its secret.
/// Returns the new service id. Used by the BaaS CRUD endpoint and by the app-create
/// `enableBaas` shortcut.
pub async fn create_baas_service(
    pool: &PgPool,
    ws_id: Uuid,
    project_id: Uuid,
    name: &str,
) -> Result<Uuid, AppError> {
    let id = Uuid::new_v4();
    let slug = format!("{}-{}", slugify(name), &id.to_string()[..8]);
    sqlx::query!(
        "INSERT INTO baas_services (id, workspace_id, project_id, name, slug) VALUES ($1, $2, $3, $4, $5)",
        id, ws_id, project_id, name.trim(), slug
    )
    .execute(pool)
    .await?;
    // Generate + publish the signing secret immediately so the pool var exists.
    rotate_secret(pool, id, ws_id, project_id).await?;
    Ok(id)
}

/// Republish `HERMES_AUTH_APP_ID` (= service id) and `HERMES_BAAS_URL`/`HERMES_APP_ID`
/// for every BaaS service, so the values match the new `/baas/:id` routes after the
/// app→service migration (those vars hold encrypted values the SQL migration can't
/// rewrite). Idempotent; safe to run on every boot.
pub async fn reconcile_baas_published_ids(pool: &PgPool) {
    let rows = match sqlx::query!(
        "SELECT id, workspace_id, project_id FROM baas_services"
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return,
    };
    for r in rows {
        let _ = publish_baas_var(pool, r.workspace_id, r.project_id, r.id, "HERMES_AUTH_APP_ID", &r.id.to_string(), false).await;
        let _ = publish_baas_var(pool, r.workspace_id, r.project_id, r.id, "HERMES_APP_ID", &r.id.to_string(), false).await;
    }
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
