//! Resolving the effective environment for an app instance.
//!
//! An instance's environment is the union of:
//!   1. project-pool vars it has opted into (live links — always read fresh), and
//!   2. its own instance-level vars.
//!
//! The instance's own value wins on a key conflict. Values are returned decrypted
//! and sorted by key for deterministic output (so e.g. the build-time ENV block
//! stays cache-stable).

use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

use crate::utils::crypto;
use crate::utils::error::AppError;

/// Sanitize an arbitrary resource name into a valid env-key fragment
/// (uppercase, non-alphanumeric -> underscore). Falls back to `fallback`.
pub fn sanitize_key_fragment(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .trim()
        .to_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim_matches('_').to_string();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed
    }
}

/// Publish a resource-owned variable into a project's env pool.
///
/// Identity is the owning resource — `(project_id, source, source_id)` — not the
/// key. If this resource already published a var, its value is refreshed in place
/// while the key is left untouched, so a key the user renamed in the Environments
/// UI survives later republishes. Only the first publish uses `key` (the suggested
/// default). Existing app links are always kept intact (live reference). Returns
/// the project_env id.
pub async fn publish_project_env(
    pool: &PgPool,
    workspace_id: Uuid,
    project_id: Uuid,
    key: &str,
    value: &str,
    is_secret: bool,
    source: &str,
    source_id: Uuid,
) -> Result<Uuid, AppError> {
    let (enc, nonce) = crypto::encrypt_env_value(value)?;

    // Already published by this resource? Refresh value/secrecy, keep the key.
    if let Some(existing) = sqlx::query_scalar!(
        "SELECT id FROM project_env_variables WHERE project_id = $1 AND source = $2 AND source_id = $3",
        project_id, source, source_id
    )
    .fetch_optional(pool)
    .await?
    {
        sqlx::query!(
            "UPDATE project_env_variables
             SET encrypted_value = $1, nonce = $2, is_secret = $3, updated_at = now()
             WHERE id = $4",
            enc, nonce, is_secret, existing
        )
        .execute(pool)
        .await?;
        return Ok(existing);
    }

    // First publish: insert under the suggested key. A pre-existing manual var on
    // the same key is taken over by this resource (matches prior behavior).
    let id = sqlx::query_scalar!(
        "INSERT INTO project_env_variables (id, workspace_id, project_id, key, encrypted_value, nonce, is_secret, source, source_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (project_id, key)
         DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, source = $8, source_id = $9, updated_at = now()
         RETURNING id",
        Uuid::new_v4(), workspace_id, project_id, key, enc, nonce, is_secret, source, source_id
    )
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// Remove every project-pool var published by a given resource. Returns the
/// running instances that had linked them, so the caller can hot-reload.
pub async fn unpublish_project_env(pool: &PgPool, source: &str, source_id: Uuid) -> Vec<Uuid> {
    let linked: Vec<Uuid> = sqlx::query_scalar!(
        "SELECT ael.app_instance_id FROM app_env_links ael
         JOIN project_env_variables pev ON pev.id = ael.project_env_id
         WHERE pev.source = $1 AND pev.source_id = $2",
        source,
        source_id
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let _ = sqlx::query!(
        "DELETE FROM project_env_variables WHERE source = $1 AND source_id = $2",
        source,
        source_id
    )
    .execute(pool)
    .await;

    linked
}

fn finalize(map: HashMap<String, String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = map.into_iter().collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Full effective env (secret + non-secret): linked project vars merged with the
/// instance's own vars, instance wins on conflict.
pub async fn resolve_instance_env(pool: &PgPool, instance_id: Uuid) -> Vec<(String, String)> {
    let mut map: HashMap<String, String> = HashMap::new();

    if let Ok(rows) = sqlx::query!(
        "SELECT pev.key, pev.encrypted_value, pev.nonce
         FROM app_env_links ael
         JOIN project_env_variables pev ON pev.id = ael.project_env_id
         WHERE ael.app_instance_id = $1",
        instance_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    if let Ok(rows) = sqlx::query!(
        "SELECT key, encrypted_value, nonce FROM environment_variables WHERE app_instance_id = $1",
        instance_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    finalize(map)
}

/// Full effective env for a cron job: project-pool vars it links merged with the
/// cron's own custom vars, the cron's own value winning on a key conflict. Mirrors
/// `resolve_instance_env` but keyed on `cron_job_id`.
pub async fn resolve_cron_env(pool: &PgPool, cron_job_id: Uuid) -> Vec<(String, String)> {
    let mut map: HashMap<String, String> = HashMap::new();

    if let Ok(rows) = sqlx::query!(
        "SELECT pev.key, pev.encrypted_value, pev.nonce
         FROM cron_env_links cel
         JOIN project_env_variables pev ON pev.id = cel.project_env_id
         WHERE cel.cron_job_id = $1",
        cron_job_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    if let Ok(rows) = sqlx::query!(
        "SELECT key, encrypted_value, nonce FROM cron_env_variables WHERE cron_job_id = $1",
        cron_job_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    finalize(map)
}

/// Non-secret subset of the effective env, for baking into the build image as
/// `ENV` (so build tooling like Vite/Next can read it). Secrets are excluded.
pub async fn resolve_instance_build_env(pool: &PgPool, instance_id: Uuid) -> Vec<(String, String)> {
    let mut map: HashMap<String, String> = HashMap::new();

    if let Ok(rows) = sqlx::query!(
        "SELECT pev.key, pev.encrypted_value, pev.nonce
         FROM app_env_links ael
         JOIN project_env_variables pev ON pev.id = ael.project_env_id
         WHERE ael.app_instance_id = $1 AND pev.is_secret = false",
        instance_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    if let Ok(rows) = sqlx::query!(
        "SELECT key, encrypted_value, nonce FROM environment_variables
         WHERE app_instance_id = $1 AND is_secret = false",
        instance_id
    )
    .fetch_all(pool)
    .await
    {
        for r in rows {
            if let Ok(v) = crypto::decrypt_env_value(&r.encrypted_value, &r.nonce) {
                map.insert(r.key, v);
            }
        }
    }

    finalize(map)
}
