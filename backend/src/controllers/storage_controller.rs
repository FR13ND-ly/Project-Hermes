use axum::{
    body::Body,
    extract::{Path, State, Query},
    http::StatusCode,
    response::sse::{Event, Sse},
    Json,
};
use futures_util::stream::StreamExt;
use uuid::Uuid;
use chrono::Utc;
use std::fs;
use std::io::Write;
use std::path::Path as StdPath;
use std::convert::Infallible;

use crate::app_state::AppState;
use crate::models::storage_model::{StorageBucket, StorageObject, StorageStatus, CompressionType, FileMetaData, BucketAccessType, BucketProcessingRules, ImageFormatTarget};
use crate::dtos::storage_dto::{CreateBucketRequest, BucketResponse, InitUploadRequest, InitUploadResponse, ObjectResponse, UpdateBucketRequest};
use crate::middlewares::auth_middleware::AuthenticatedUser;
use crate::utils::{storage_engine::StorageEngine, error::AppError};
use crate::utils::pagination::{PaginationParams, Paginated};
use image::io::Reader as ImageReader;

enum StorageAuth {
    PlatformUser {
        claims: crate::middlewares::auth_middleware::Claims,
        ws_id: Uuid,
    },
    BucketCredentials {
        bucket: StorageBucket,
        ws_id: Uuid,
    },
}

async fn check_user_permission(
    pool: &sqlx::PgPool,
    claims: &crate::middlewares::auth_middleware::Claims,
    ws_id: Uuid,
    permission: &str,
) -> Result<(), AppError> {
    if claims.is_super_admin {
        return Ok(());
    }

    let has_perm = sqlx::query_scalar!(
        "SELECT EXISTS (
            SELECT 1 
            FROM workspace_members wm
            JOIN role_permissions rp ON wm.role_id = rp.role_id
            JOIN permissions p ON rp.permission_id = p.id
            WHERE wm.workspace_id = $1 
              AND wm.user_id = $2 
              AND p.name = $3
        ) as \"has_perm!\"",
        ws_id,
        claims.sub,
        permission
    )
    .fetch_one(pool)
    .await?;

    if !has_perm {
        return Err(AppError::Permission(format!(
            "Permission '{}' required for this workspace.",
            permission
        )));
    }

    Ok(())
}

async fn authenticate_request(
    pool: &sqlx::PgPool,
    headers: &axum::http::HeaderMap,
    query_token: Option<&str>,
    query_app_id: Option<&str>,
    bucket_slug_hint: Option<&str>,
    bucket_id_hint: Option<Uuid>,
) -> Result<StorageAuth, AppError> {
    // 1. Try to extract app_id and secret_key from custom headers
    let header_app_id = headers
        .get("x-hermes-app-id")
        .or_else(|| headers.get("hermes-app-id"))
        .and_then(|v| v.to_str().ok());

    let header_secret_key = headers
        .get("x-hermes-secret-key")
        .or_else(|| headers.get("hermes-secret-key"))
        .or_else(|| headers.get("x-hermes-api-key"))
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // 2. Extract token/secret from Authorization header or query parameter
    let token = if let Some(sk) = header_secret_key {
        sk
    } else if let Some(auth_header) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if !auth_header.starts_with("Bearer ") {
            return Err(AppError::Auth("Invalid Authorization header format".to_string()));
        }
        auth_header[7..].to_string()
    } else if let Some(t) = query_token {
        t.to_string()
    } else {
        return Err(AppError::Auth("Missing Authorization token or secret key".to_string()));
    };

    // 3. Try parsing token as JWT
    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "super_secret_key".to_string());
    if let Ok(token_data) = jsonwebtoken::decode::<crate::middlewares::auth_middleware::Claims>(
        &token,
        &jsonwebtoken::DecodingKey::from_secret(jwt_secret.as_bytes()),
        &jsonwebtoken::Validation::default(),
    ) {
        let claims = token_data.claims;
        if claims.status == crate::models::user_model::UserStatus::Suspended {
            return Err(AppError::Permission("This account has been suspended".to_string()));
        }
        if claims.status == crate::models::user_model::UserStatus::PendingVerification {
            return Err(AppError::Permission("Account activation required".to_string()));
        }
        let ws_id = claims.current_workspace_id.ok_or_else(|| {
            AppError::Validation("No active workspace selected.".to_string())
        })?;
        return Ok(StorageAuth::PlatformUser { claims, ws_id });
    }

    // 4. Fallback: Check as bucket credentials using app_id and secret key
    let resolved_app_id = header_app_id.or(query_app_id);

    if let Some(app_id_val) = resolved_app_id {
        let bucket = sqlx::query_as::<_, StorageBucket>(
            "SELECT * FROM storage_buckets WHERE app_id = $1"
        )
        .bind(app_id_val)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Bucket not found for provided app_id.".to_string()))?;

        if let (Some(e), Some(n)) = (&bucket.secret_key_encrypted, &bucket.secret_key_nonce) {
            if let Ok(decrypted) = crate::utils::crypto::decrypt_env_value(e, n) {
                if decrypted == token {
                    let ws_id = bucket.workspace_id;
                    return Ok(StorageAuth::BucketCredentials { bucket, ws_id });
                }
            }
        }
    } else if let Some(bucket_id) = bucket_id_hint {
        let bucket = sqlx::query_as::<_, StorageBucket>(
            "SELECT * FROM storage_buckets WHERE id = $1"
        )
        .bind(bucket_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("Bucket not found.".to_string()))?;

        if let (Some(e), Some(n)) = (&bucket.secret_key_encrypted, &bucket.secret_key_nonce) {
            if let Ok(decrypted) = crate::utils::crypto::decrypt_env_value(e, n) {
                if decrypted == token {
                    let ws_id = bucket.workspace_id;
                    return Ok(StorageAuth::BucketCredentials { bucket, ws_id });
                }
            }
        }
    } else if let Some(slug) = bucket_slug_hint {
        let buckets = sqlx::query_as::<_, StorageBucket>(
            "SELECT * FROM storage_buckets WHERE slug = $1"
        )
        .bind(slug)
        .fetch_all(pool)
        .await?;

        for bucket in buckets {
            if let (Some(e), Some(n)) = (&bucket.secret_key_encrypted, &bucket.secret_key_nonce) {
                if let Ok(decrypted) = crate::utils::crypto::decrypt_env_value(e, n) {
                    if decrypted == token {
                        let ws_id = bucket.workspace_id;
                        return Ok(StorageAuth::BucketCredentials { bucket, ws_id });
                    }
                }
            }
        }
    }

    Err(AppError::Auth("Invalid or expired token".to_string()))
}

/// Upsert a bucket's access credentials into the project env pool as two vars:
/// <PREFIX>_APP_ID (visible) and <PREFIX>_SECRET_KEY (secret). Both carry
/// source='storage'/source_id=bucket so delete_bucket removes them together.
async fn publish_bucket_credentials(
    pool: &sqlx::PgPool,
    ws_id: Uuid,
    project_id: Uuid,
    bucket_id: Uuid,
    prefix: &str,
    app_id: &str,
    secret_key: &str,
) {
    // If this bucket already published its credentials, refresh the VALUES in place
    // (identity = the owning bucket, not the key) so the published key NAMES never
    // drift across create/rotate/reconcile — the APP_ID row is the non-secret one,
    // the SECRET_KEY row the secret one. This mirrors how databases publish via
    // app_env::publish_project_env and fixes both "the value never updates" and
    // "rotation republishes under a different name than the convention".
    let existing = sqlx::query!(
        "SELECT id, is_secret FROM project_env_variables WHERE project_id = $1 AND source = 'storage' AND source_id = $2",
        project_id, bucket_id
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let has_app_id = existing.iter().any(|r| !r.is_secret);
    let has_secret = existing.iter().any(|r| r.is_secret);

    if has_app_id && has_secret {
        for r in existing {
            let value = if r.is_secret { secret_key } else { app_id };
            if let Ok((enc, nonce)) = crate::utils::crypto::encrypt_env_value(value) {
                let _ = sqlx::query!(
                    "UPDATE project_env_variables SET encrypted_value = $1, nonce = $2, updated_at = now() WHERE id = $3",
                    enc, nonce, r.id
                )
                .execute(pool)
                .await;
            }
        }
        return;
    }

    // First publish for this bucket — create the two canonical keys.
    for (key, value, is_secret) in [
        (format!("{}_APP_ID", prefix), app_id.to_string(), false),
        (format!("{}_SECRET_KEY", prefix), secret_key.to_string(), true),
    ] {
        if let Ok((enc, nonce)) = crate::utils::crypto::encrypt_env_value(&value) {
            let _ = sqlx::query!(
                "INSERT INTO project_env_variables (id, workspace_id, project_id, key, encrypted_value, nonce, is_secret, source, source_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'storage', $8)
                 ON CONFLICT (project_id, key)
                 DO UPDATE SET encrypted_value = $5, nonce = $6, is_secret = $7, source = 'storage', source_id = $8, updated_at = now()",
                Uuid::new_v4(), ws_id, project_id, key, enc, nonce, is_secret, bucket_id
            )
            .execute(pool)
            .await;
        }
    }
}

/// Backfill access credentials for buckets created before the key-pair model, and
/// publish them for project-scoped buckets. Idempotent — runs on every startup.
pub async fn reconcile_bucket_credentials(pool: &sqlx::PgPool) {
    let rows = match sqlx::query!(
        "SELECT id, workspace_id, project_id, slug FROM storage_buckets WHERE app_id IS NULL"
    )
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(_) => return,
    };
    for b in rows {
        let app_id = format!("hsk_{}", crate::utils::string_gen::generate_secure_string(24));
        let secret_key = crate::utils::string_gen::generate_secure_string(40);
        let (enc, nonce) = match crate::utils::crypto::encrypt_env_value(&secret_key) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let updated = sqlx::query!(
            "UPDATE storage_buckets SET app_id = $1, secret_key_encrypted = $2, secret_key_nonce = $3 WHERE id = $4 AND app_id IS NULL",
            app_id, enc, nonce, b.id
        )
        .execute(pool)
        .await;
        if updated.map(|r| r.rows_affected() > 0).unwrap_or(false) {
            if let Some(pid) = b.project_id {
                let prefix = format!("BUCKET_{}", crate::utils::app_env::sanitize_key_fragment(&b.slug, "STORAGE"));
                publish_bucket_credentials(pool, b.workspace_id, pid, b.id, &prefix, &app_id, &secret_key).await;
            }
        }
    }
}

/// POST /buckets/:id/rotate-credentials — regenerate the bucket's `secret_key`
/// (the `app_id` identity stays stable, mirroring access-key-id/secret), re-encrypt,
/// persist, and republish to the project env pool. Returns the new secret_key (shown
/// once). Apps holding the old key break until they reload — the UI warns, no auto-reload.
pub async fn rotate_bucket_credentials(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(bucket_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let bucket = sqlx::query!(
        "SELECT slug, project_id, app_id FROM storage_buckets WHERE id = $1 AND workspace_id = $2",
        bucket_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Storage bucket not found.".to_string()))?;

    // Keep app_id stable (identity); rotate only the secret.
    let app_id = bucket
        .app_id
        .unwrap_or_else(|| format!("hsk_{}", crate::utils::string_gen::generate_secure_string(24)));
    let secret_key = crate::utils::string_gen::generate_secure_string(40);
    let (enc, nonce) = crate::utils::crypto::encrypt_env_value(&secret_key)?;

    sqlx::query!(
        "UPDATE storage_buckets SET app_id = $1, secret_key_encrypted = $2, secret_key_nonce = $3, updated_at = now() WHERE id = $4",
        app_id, enc, nonce, bucket_id
    )
    .execute(&state.pool)
    .await?;

    if let Some(pid) = bucket.project_id {
        let prefix = format!("BUCKET_{}", crate::utils::app_env::sanitize_key_fragment(&bucket.slug, "STORAGE"));
        publish_bucket_credentials(&state.pool, ws_id, pid, bucket_id, &prefix, &app_id, &secret_key).await;

        // Reload every app linked to this bucket's pool vars so the rotated secret
        // actually reaches running consumers (not just the pool).
        let linked: std::collections::HashSet<Uuid> = sqlx::query_scalar!(
            "SELECT ael.app_instance_id FROM app_env_links ael
             JOIN project_env_variables pev ON pev.id = ael.project_env_id
             WHERE pev.source = 'storage' AND pev.source_id = $1",
            bucket_id
        )
        .fetch_all(&state.pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();
        for inst in linked {
            crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
        }
    }

    Ok(Json(serde_json::json!({ "app_id": app_id, "secret_key": secret_key })))
}

pub async fn create_bucket(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<CreateBucketRequest>,
) -> Result<(StatusCode, Json<BucketResponse>), AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let slug = payload.name.trim().to_lowercase().replace(' ', "-");
    let is_public = payload.is_public.unwrap_or(false);
    // 0 = unlimited; limits are opt-in (set explicitly per bucket), matching the
    // workspace RAM/storage convention.
    let max_size = payload.max_bucket_size_bytes.unwrap_or(0);
    let max_file_size = payload.max_file_size_bytes.unwrap_or(0);
    let allow_custom_processing = payload.allow_custom_processing.unwrap_or(false);
    let processing_rules = payload.default_processing_rules.unwrap_or_default();

    let slug_exists = sqlx::query_scalar!(
        "SELECT EXISTS(SELECT 1 FROM storage_buckets WHERE workspace_id = $1 AND slug = $2)",
        ws_id, slug
    )
    .fetch_one(&state.pool)
    .await?
    .unwrap_or(false);

    if slug_exists {
        return Err(AppError::Conflict("A storage bucket with this name already exists.".to_string()));
    }

    let base_domain = std::env::var("HERMES_BASE_DOMAIN").unwrap_or_else(|_| "hermes-host.vip".to_string());

    // Buckets are private-only now (no static-website / public-assets types).
    let access_type = BucketAccessType::PrivateStorage;

    let mut tx = state.pool.begin().await?;
    let bucket_id = Uuid::new_v4();
    let bucket_dir = StorageEngine::get_bucket_path(&ws_id.to_string(), &slug, &access_type);

    // Per-bucket access credentials. Apps connect with (app_id, secret_key).
    let app_id = format!("hsk_{}", crate::utils::string_gen::generate_secure_string(24));
    let secret_key = crate::utils::string_gen::generate_secure_string(40);
    let (secret_enc, secret_nonce) = crate::utils::crypto::encrypt_env_value(&secret_key)?;

    sqlx::query!(
        "INSERT INTO storage_buckets (id, workspace_id, project_id, name, slug, access_type, is_public, max_bucket_size_bytes, max_file_size_bytes, allow_custom_processing, default_processing_rules, created_by, app_id, secret_key_encrypted, secret_key_nonce)
         VALUES ($1, $2, $3, $4, $5, $6::bucket_access_type, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        bucket_id, ws_id, payload.project_id, payload.name.trim(), slug, access_type as _, is_public, max_size, max_file_size, allow_custom_processing, sqlx::types::Json(processing_rules.clone()) as _, claims.sub, app_id, secret_enc, secret_nonce
    )
    .execute(&mut *tx)
    .await?;

    let assigned_domain: Option<String> = None;
    fs::create_dir_all(&bucket_dir)
        .map_err(|e| AppError::Infrastructure(format!("Failed to create bucket directory: {}", e)))?;

    tx.commit().await?;

    // Publish the bucket's access credentials into the project env pool when scoped
    // to a project. Both BUCKET_<SLUG>_APP_ID (visible) and BUCKET_<SLUG>_SECRET_KEY
    // (secret) are added so apps can connect with the key pair. Opt-out via
    // publish_to_env=false; an optional env_key prefix may be supplied.
    let _ = base_domain; // legacy public-URL publish removed (buckets are private)
    if let (Some(pid), true) = (payload.project_id, payload.publish_to_env.unwrap_or(true)) {
        let in_workspace = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
            pid, ws_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
        if in_workspace {
            let prefix = match payload.env_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(custom) => crate::utils::app_env::sanitize_key_fragment(custom, &format!("BUCKET_{}", crate::utils::app_env::sanitize_key_fragment(&slug, "STORAGE"))),
                None => format!("BUCKET_{}", crate::utils::app_env::sanitize_key_fragment(&slug, "STORAGE")),
            };
            publish_bucket_credentials(&state.pool, ws_id, pid, bucket_id, &prefix, &app_id, &secret_key).await;
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(BucketResponse {
            id: bucket_id,
            name: payload.name,
            slug,
            access_type,
            is_public,
            assigned_domain,
            allowed_file_types: payload.allowed_file_types,
            max_bucket_size_bytes: max_size,
            max_file_size_bytes: max_file_size,
            allow_custom_processing,
            default_processing_rules: processing_rules,
            created_at: Utc::now(),
        }),
    ))
}

pub async fn initialize_upload(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<InitUploadRequest>,
) -> Result<Json<InitUploadResponse>, AppError> {
    let clean_path = payload.file_path.trim().trim_start_matches('/').to_string();
    let bucket_slug = clean_path.split('/').next().ok_or_else(|| {
        AppError::Validation("Invalid file path format. Must include bucket prefix.".to_string())
    })?;

    let relative_file_path = clean_path.strip_prefix(bucket_slug).unwrap_or(&clean_path).trim_start_matches('/').to_string();

    if relative_file_path.is_empty() {
        return Err(AppError::Validation("File path cannot be empty after bucket resolution.".to_string()));
    }

    let auth = authenticate_request(
        &state.pool,
        &headers,
        None,
        None,
        Some(bucket_slug),
        None,
    ).await?;

    let (bucket, ws_id) = match auth {
        StorageAuth::PlatformUser { claims, ws_id } => {
            check_user_permission(&state.pool, &claims, ws_id, "volume:create").await?;

            let b = sqlx::query_as::<_, StorageBucket>(
                "SELECT * FROM storage_buckets WHERE workspace_id = $1 AND slug = $2"
            )
            .bind(ws_id)
            .bind(bucket_slug)
            .fetch_optional(&state.pool)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("Target bucket '{}' not found.", bucket_slug)))?;

            (b, ws_id)
        }
        StorageAuth::BucketCredentials { bucket, ws_id } => {
            (bucket, ws_id)
        }
    };

    // Enforce the per-file size limit (0 = unlimited).
    if bucket.max_file_size_bytes > 0 && payload.size_bytes > bucket.max_file_size_bytes {
        return Err(AppError::Validation(format!(
            "File size {} exceeds the per-file limit of {} for this bucket.",
            format_bytes(payload.size_bytes),
            format_bytes(bucket.max_file_size_bytes)
        )));
    }

    // Enforce the bucket's total-size limit (0 = unlimited). Counts all existing
    // objects (conservative on overwrite — the prior copy is replaced just below).
    if bucket.max_bucket_size_bytes > 0 {
        let used: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM storage_objects WHERE bucket_id = $1",
            bucket.id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(0);
        if used + payload.size_bytes > bucket.max_bucket_size_bytes {
            return Err(AppError::Validation(format!(
                "Bucket is full: {} of {} used; this {} upload would exceed the limit.",
                format_bytes(used),
                format_bytes(bucket.max_bucket_size_bytes),
                format_bytes(payload.size_bytes)
            )));
        }
    }

    // Enforce the workspace-wide native-storage quota (max_storage_gb; 0 = unlimited).
    // The k8s ResourceQuota only counts PVCs (app volumes / DB storage); native
    // buckets live on host disk and would otherwise escape the workspace cap.
    let ws_max_storage_gb: i32 = sqlx::query_scalar!(
        "SELECT max_storage_gb FROM workspaces WHERE id = $1",
        ws_id
    )
    .fetch_one(&state.pool)
    .await?;
    if ws_max_storage_gb > 0 {
        let ws_max_bytes = ws_max_storage_gb as i64 * 1_073_741_824;
        let ws_used: i64 = sqlx::query_scalar!(
            "SELECT COALESCE(SUM(o.size_bytes), 0)::bigint
             FROM storage_objects o JOIN storage_buckets b ON o.bucket_id = b.id
             WHERE b.workspace_id = $1",
            ws_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(0);
        if ws_used + payload.size_bytes > ws_max_bytes {
            return Err(AppError::Validation(format!(
                "Workspace storage is full: {} of {} used; this {} upload would exceed the workspace limit.",
                format_bytes(ws_used),
                format_bytes(ws_max_bytes),
                format_bytes(payload.size_bytes)
            )));
        }
    }

    if let Some(allowed) = bucket.allowed_file_types {
        if !allowed.contains(&payload.mime_type) {
            return Err(AppError::Validation(format!("Mime-type '{}' is not allowed.", payload.mime_type)));
        }
    }

    // Per-upload processing overrides are honored only when the bucket opts in;
    // otherwise the bucket's default rules always win (client options ignored).
    let final_processing_options = match (bucket.allow_custom_processing, payload.custom_processing_options) {
        (true, Some(custom)) => custom,
        _ => bucket.default_processing_rules.0,
    };

    // Check if an object with the same name already exists in this bucket
    let existing_object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE bucket_id = $1 AND file_path = $2"
    )
    .bind(bucket.id)
    .bind(&relative_file_path)
    .fetch_optional(&state.pool)
    .await?;

    if let Some(obj) = existing_object {
        // Physical deletion of variants and compression files
        let _ = StorageEngine::delete_object_physical(
            &ws_id.to_string(),
            &bucket_slug,
            &bucket.access_type,
            &obj.file_path,
            obj.compression,
            &obj.meta_data.0.variants,
        ).await;

        // Delete from database
        sqlx::query!("DELETE FROM storage_objects WHERE id = $1", obj.id)
            .execute(&state.pool)
            .await?;
    }

    let file_id = Uuid::new_v4();
    let etag = format!("{:x}", md5::compute(&relative_file_path));
    let default_meta = sqlx::types::Json(FileMetaData {
        has_variants: false,
        original_extension: StdPath::new(&relative_file_path).extension().map(|e| e.to_string_lossy().to_string()),
        variants: None,
        error_reason: None,
    });

    sqlx::query!(
        "INSERT INTO storage_objects (id, bucket_id, file_path, size_bytes, mime_type, etag, status, meta_data, processing_options)
         VALUES ($1, $2, $3, $4, $5, $6, 'pending_upload'::storage_status, $7, $8)",
        file_id, bucket.id, relative_file_path, payload.size_bytes, payload.mime_type, etag, default_meta as _, sqlx::types::Json(final_processing_options) as _
    )
    .execute(&state.pool)
    .await?;

    Ok(Json(InitUploadResponse {
        file_id,
        status: StorageStatus::PendingUpload,
        upload_url: format!("/storage/upload/{}", file_id),
    }))
}

#[tracing::instrument(skip_all, fields(file_id = %file_id), err)]
pub async fn process_upload_stream(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
    body: Body,
) -> Result<Json<ObjectResponse>, AppError> {
    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE id = $1 AND status = 'pending_upload'::storage_status"
    )
    .bind(file_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Upload session not found or already processed.".to_string()))?;

    let (workspace_id, bucket_slug, access_type): (Uuid, String, BucketAccessType) = sqlx::query_as(
        "SELECT workspace_id, slug, access_type FROM storage_buckets WHERE id = $1"
    )
    .bind(object.bucket_id)
    .fetch_one(&state.pool)
    .await?;

    let bucket_dir = StorageEngine::get_bucket_path(&workspace_id.to_string(), &bucket_slug, &access_type);
    let final_disk_path = bucket_dir.join(&object.file_path);

    if let Some(parent) = final_disk_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(&final_disk_path)
        .map_err(|e| AppError::Infrastructure(format!("Cannot create file on host: {}", e)))?;

    let total_size = object.size_bytes as f64;
    let mut uploaded_bytes = 0.0;
    let mut hasher = md5::Context::new();
    let mut stream = body.into_data_stream();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| AppError::Infrastructure(format!("Network stream error: {}", e)))?;
        
        file.write_all(&chunk)?;
        hasher.consume(&chunk);

        uploaded_bytes += chunk.len() as f64;
        
        let percent = if total_size > 0.0 {
            ((uploaded_bytes / total_size) * 100.0).round() as i32
        } else {
            100
        };

        let _ = state.progress_tx.send((file_id, percent));
    }

    file.flush()?;
    let real_etag = format!("{:x}", hasher.compute());

    sqlx::query!(
        "UPDATE storage_objects SET status = 'processing'::storage_status, etag = $1 WHERE id = $2",
        real_etag, file_id
    )
    .execute(&state.pool)
    .await?;

    let pool_clone = state.pool.clone();
    let ws_str = workspace_id.to_string();
    let slug_str = bucket_slug.clone();
    let relative_path = object.file_path.clone();
    let mime_type = object.mime_type.clone();
    let disk_path_clone = final_disk_path.clone();
    let options = object.processing_options.0;
    let event_bucket_id = object.bucket_id;

    tokio::spawn(async move {
        // Granular processing stages are reported through an unbounded channel and
        // drained into `storage_objects.processing_stage` concurrently, so the
        // dashboard's 2s poll can surface "Variantă: thumb", "Conversie", etc. live.
        let (stage_tx, mut stage_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let drain_pool = pool_clone.clone();
        let drain_handle = tokio::spawn(async move {
            while let Some(stage) = stage_rx.recv().await {
                let _ = sqlx::query!(
                    "UPDATE storage_objects SET processing_stage = $1 WHERE id = $2",
                    stage, file_id
                )
                .execute(&drain_pool)
                .await;
            }
        });
        let _ = stage_tx.send("analyzing".to_string());

        let mut meta = FileMetaData {
            has_variants: false,
            original_extension: StdPath::new(&relative_path).extension().map(|e| e.to_string_lossy().to_string()),
            variants: None,
            error_reason: None,
        };

        let mut compression_mode = CompressionType::None;
        let mut original_size = None;
        let mut is_optimized = false;
        let mut dimensions = None;

        let mut final_relative_path = relative_path.clone();
        let mut final_mime_type = mime_type.clone();
        let mut final_size_bytes = fs::metadata(&disk_path_clone).map(|m| m.len() as i64).unwrap_or(0);

        let processing_result = (|| -> Result<(), AppError> {
            if mime_type.starts_with("image/") && mime_type != "image/gif" {
                if let Some(img_rules) = options.image_options {
                    let mut current_disk_path = disk_path_clone.clone();
                    let mut current_relative_path = relative_path.clone();
                    let mut current_mime_type = mime_type.clone();

                    if img_rules.convert_to != ImageFormatTarget::Original {
                        let _ = stage_tx.send("converting".to_string());
                        let target_ext = match img_rules.convert_to {
                            ImageFormatTarget::Webp => "webp",
                            ImageFormatTarget::Avif => "avif",
                            ImageFormatTarget::Jpg => "jpg",
                            ImageFormatTarget::Original => unreachable!(),
                        };

                        let target_mime = match img_rules.convert_to {
                            ImageFormatTarget::Webp => "image/webp",
                            ImageFormatTarget::Avif => "image/avif",
                            ImageFormatTarget::Jpg => "image/jpeg",
                            ImageFormatTarget::Original => unreachable!(),
                        };

                        let img = ImageReader::open(&disk_path_clone)
                            .map_err(|e| AppError::Validation(format!("Invalid image file format: {}", e)))?
                            .decode()
                            .map_err(|e| AppError::Fatal(anyhow::anyhow!("Image decoding engine crashed: {}", e)))?;

                        let new_relative_path = if let Some(ext) = StdPath::new(&relative_path).extension() {
                            let ext_str = ext.to_string_lossy();
                            relative_path.strip_suffix(&*ext_str)
                                .map(|s| format!("{}{}", s, target_ext))
                                .unwrap_or_else(|| format!("{}.{}", relative_path, target_ext))
                        } else {
                            format!("{}.{}", relative_path, target_ext)
                        };

                        let bucket_dir = StorageEngine::get_bucket_path(&ws_str, &slug_str, &access_type);
                        let new_disk_path = bucket_dir.join(&new_relative_path);

                        StorageEngine::save_image_with_options(&img, &new_disk_path, img_rules.quality)?;

                        if new_disk_path != disk_path_clone {
                            let _ = fs::remove_file(&disk_path_clone);
                        }

                        current_disk_path = new_disk_path;
                        current_relative_path = new_relative_path;
                        current_mime_type = target_mime.to_string();
                    }

                    let (orig_dims, image_variants) = StorageEngine::generate_image_variants_smart(
                        &ws_str,
                        &slug_str,
                        &access_type,
                        &current_relative_path,
                        &img_rules,
                        Some(&stage_tx),
                    )?;

                    dimensions = Some(orig_dims);
                    meta.has_variants = !image_variants.is_empty();
                    meta.variants = Some(image_variants);
                    is_optimized = true;

                    final_relative_path = current_relative_path;
                    final_mime_type = current_mime_type;
                    final_size_bytes = fs::metadata(&current_disk_path).map(|m| m.len() as i64).unwrap_or(0);
                }
            } else if mime_type == "application/javascript" || mime_type == "text/css" || mime_type == "text/html" {
                if let Some(text_rules) = options.text_options {
                    if text_rules.pre_compress_brotli {
                        let _ = stage_tx.send("compressing".to_string());
                        compression_mode = CompressionType::Brotli;
                        let size_on_disk = fs::metadata(&disk_path_clone).map(|m| m.len() as i64).unwrap_or(0);
                        original_size = Some(size_on_disk);
                        StorageEngine::compress_file(&disk_path_clone, CompressionType::Brotli)?;
                    }
                }
            }
            Ok(())
        })();

        // Close the stage channel and let the drain flush any queued stage updates
        // before the terminal UPDATE clears processing_stage.
        drop(stage_tx);
        let _ = drain_handle.await;

        let final_status = match processing_result {
            Ok(_) => StorageStatus::Ready,
            Err(e) => {
                meta.error_reason = Some(format!("{:?}", e));
                StorageStatus::Failed
            }
        };

        let meta_clone = meta.clone();
        let _ = sqlx::query!(
            "UPDATE storage_objects
             SET status = $1::storage_status,
                 compression = $2::compression_type,
                 original_size_bytes = $3,
                 is_optimized = $4,
                 image_dimensions = $5,
                 meta_data = $6,
                 file_path = $7,
                 mime_type = $8,
                 size_bytes = $9,
                 processing_stage = NULL,
                 updated_at = now()
             WHERE id = $10",
            final_status as _,
            compression_mode as _,
            original_size,
            is_optimized,
            dimensions,
            sqlx::types::Json(meta) as _,
            final_relative_path,
            final_mime_type,
            final_size_bytes,
            file_id
        )
        .execute(&pool_clone)
        .await;

        if final_status == StorageStatus::Ready {
            let _ = crate::utils::storage_engine::StorageEngine::sync_object_to_s3_and_cleanup(
                &ws_str,
                &slug_str,
                &access_type,
                &final_relative_path,
                compression_mode,
                &meta_clone.variants,
            ).await;
        }

        // Notify the dashboard live that processing finished (Ready/Failed) so the
        // Storage UI refreshes instantly instead of waiting for the safety-net poll.
        crate::utils::event_broadcaster::broadcast_event(
            crate::utils::event_broadcaster::SystemEvent::StorageObjectUpdated {
                workspace_id,
                bucket_id: event_bucket_id,
                object_id: file_id,
                status: format!("{:?}", final_status).to_lowercase(),
            }
        );
    });

    let virtual_url = calculate_virtual_url(object.id, &object.file_path, &bucket_slug, workspace_id, &access_type);

    Ok(Json(ObjectResponse {
        id: object.id,
        bucket_id: object.bucket_id,
        file_path: object.file_path,
        size_bytes: object.size_bytes,
        mime_type: object.mime_type,
        etag: real_etag,
        status: StorageStatus::Processing,
        processing_stage: None,
        compression: CompressionType::None,
        original_size_bytes: None,
        is_optimized: false,
        image_dimensions: None,
        has_variants: false,
        variants: None,
        virtual_url,
        created_at: object.created_at,
    }))
}

pub async fn upload_progress_stream(
    State(state): State<AppState>,
    Path(file_id): Path<Uuid>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.progress_tx.subscribe();

    let stream = async_stream::stream! {
        while let Ok((id, percent)) = rx.recv().await {
            if id == file_id {
                yield Ok(Event::default().data(percent.to_string()));
                if percent >= 100 {
                    break;
                }
            }
        }
    };

    Sse::new(stream)
}

#[tracing::instrument(skip_all, fields(file_id = %file_id), err)]
pub async fn download_private_file(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Path(file_id): Path<Uuid>,
) -> Result<axum::response::Response, AppError> {
    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE id = $1"
    )
    .bind(file_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Requested file not found.".to_string()))?;

    let query_token = params.get("token").or_else(|| params.get("secret_key")).map(|s| s.as_str());
    let query_app_id = params.get("app_id").map(|s| s.as_str());

    let auth = authenticate_request(
        &state.pool,
        &headers,
        query_token,
        query_app_id,
        None,
        Some(object.bucket_id),
    ).await?;

    let (ws_id, bucket_slug, access_type) = match auth {
        StorageAuth::PlatformUser { claims, ws_id } => {
            check_user_permission(&state.pool, &claims, ws_id, "volume:read").await?;

            let (bucket_ws_id, bucket_slug, access_type): (Uuid, String, BucketAccessType) = sqlx::query_as(
                "SELECT workspace_id, slug, access_type FROM storage_buckets WHERE id = $1"
            )
            .bind(object.bucket_id)
            .fetch_one(&state.pool)
            .await?;

            if bucket_ws_id != ws_id {
                return Err(AppError::Permission("You do not have permission to access this storage bucket.".to_string()));
            }

            (ws_id, bucket_slug, access_type)
        }
        StorageAuth::BucketCredentials { bucket, ws_id } => {
            (ws_id, bucket.slug, bucket.access_type)
        }
    };

    if object.status != StorageStatus::Ready {
        return Err(AppError::Validation("File is not ready for download yet.".to_string()));
    }

    let provider = std::env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "local".to_string());
    if provider == "s3" {
        let s3_bucket_name = std::env::var("S3_BUCKET")
            .map_err(|_| AppError::Infrastructure("S3_BUCKET env var not set".to_string()))?;
        let s3_endpoint = std::env::var("S3_ENDPOINT").ok();
        let s3_region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string());
        let access_key = std::env::var("AWS_ACCESS_KEY_ID")
            .map_err(|_| AppError::Infrastructure("AWS_ACCESS_KEY_ID env var not set".to_string()))?;
        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY")
            .map_err(|_| AppError::Infrastructure("AWS_SECRET_ACCESS_KEY env var not set".to_string()))?;

        let credentials = s3::creds::Credentials::new(
            Some(&access_key),
            Some(&secret_key),
            None,
            None,
            None,
        ).map_err(|e| AppError::Infrastructure(format!("Failed to parse S3 credentials: {}", e)))?;

        let region = match s3_endpoint {
            Some(endpoint) => s3::region::Region::Custom {
                region: s3_region,
                endpoint,
            },
            None => s3_region.parse().map_err(|e| AppError::Infrastructure(format!("Failed to parse S3 region: {}", e)))?,
        };

        let bucket = s3::Bucket::new(&s3_bucket_name, region, credentials)
            .map_err(|e| AppError::Infrastructure(format!("Failed to connect to S3 Bucket: {}", e)))?;

        let s3_path = format!("hermes/{}/{}/{}", ws_id, bucket_slug, object.file_path);
        
        let presigned_url = bucket.presign_get(&s3_path, 3600, None)
            .map_err(|e| AppError::Infrastructure(format!("Failed to generate S3 presigned URL: {}", e)))?;

        let response = axum::response::Response::builder()
            .status(StatusCode::FOUND)
            .header(axum::http::header::LOCATION, presigned_url)
            .body(Body::empty())
            .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to construct redirect response: {}", e)))?;

        return Ok(response);
    }

    let bucket_dir = StorageEngine::get_bucket_path(&ws_id.to_string(), &bucket_slug, &access_type);
    let full_disk_path = bucket_dir.join(&object.file_path);

    let file = tokio::fs::File::open(&full_disk_path)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to open file on host disk: {}", e)))?;

    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));

    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, object.mime_type)
        .header(axum::http::header::CONTENT_LENGTH, object.size_bytes)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{}\"", StdPath::new(&object.file_path).file_name().unwrap_or_default().to_string_lossy()),
        )
        .body(body)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to construct streaming response: {}", e)))?;

    Ok(response)
}

fn calculate_virtual_url(
    object_id: Uuid,
    _file_path: &str,
    _bucket_slug: &str,
    _workspace_id: Uuid,
    _access_type: &BucketAccessType,
) -> String {
    // All buckets are private now — access is always via the tokenized API route.
    format!("/api/v1/storage/private/{}", object_id)
}

/// Human-readable byte size for validation error messages.
fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{:.2} {}", size, UNITS[unit])
    }
}

pub async fn list_buckets(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
) -> Result<Json<Vec<BucketResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    // SELECT * so every StorageBucket field (incl. app_id / secret_key_* added
    // later) is hydrated by FromRow; the credentials are not exposed in
    // BucketResponse below, so they never reach the client.
    let buckets = sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE workspace_id = $1 ORDER BY created_at DESC"
    )
    .bind(ws_id)
    .fetch_all(&state.pool)
    .await?;

    let response = buckets
        .into_iter()
        .map(|b| {
            let assigned_domain: Option<String> = None;
            BucketResponse {
                id: b.id,
                name: b.name,
                slug: b.slug,
                access_type: b.access_type,
                is_public: b.is_public,
                assigned_domain,
                allowed_file_types: b.allowed_file_types,
                max_bucket_size_bytes: b.max_bucket_size_bytes,
                max_file_size_bytes: b.max_file_size_bytes,
                allow_custom_processing: b.allow_custom_processing,
                default_processing_rules: b.default_processing_rules.0,
                created_at: b.created_at,
            }
        })
        .collect();

    Ok(Json(response))
}

/// Physically removes every object + bucket directory for all buckets owned by a
/// project. Best-effort (errors are ignored) — the caller is responsible for
/// deleting the DB rows (storage_objects cascade from storage_buckets). Used by
/// project teardown so buckets and their files never outlive their project.
pub async fn purge_project_buckets_physical(pool: &sqlx::PgPool, ws_id: Uuid, project_id: Uuid) {
    let buckets = match sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE project_id = $1 AND workspace_id = $2"
    )
    .bind(project_id)
    .bind(ws_id)
    .fetch_all(pool)
    .await
    {
        Ok(b) => b,
        Err(_) => return,
    };

    for bucket in buckets {
        if let Ok(objects) = sqlx::query_as::<_, StorageObject>(
            "SELECT * FROM storage_objects WHERE bucket_id = $1"
        )
        .bind(bucket.id)
        .fetch_all(pool)
        .await
        {
            for o in objects {
                let _ = StorageEngine::delete_object_physical(
                    &ws_id.to_string(),
                    &bucket.slug,
                    &bucket.access_type,
                    &o.file_path,
                    o.compression,
                    &o.meta_data.0.variants,
                ).await;
            }
        }
        let _ = StorageEngine::delete_bucket_physical(&ws_id.to_string(), &bucket.slug, &bucket.access_type).await;
    }
}

pub async fn delete_bucket(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(bucket_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let bucket = sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE id = $1 AND workspace_id = $2"
    )
    .bind(bucket_id)
    .bind(ws_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Storage bucket not found.".to_string()))?;

    let objects = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE bucket_id = $1"
    )
    .bind(bucket.id)
    .fetch_all(&state.pool)
    .await?;

    for o in objects {
        let _ = StorageEngine::delete_object_physical(
            &ws_id.to_string(),
            &bucket.slug,
            &bucket.access_type,
            &o.file_path,
            o.compression,
            &o.meta_data.0.variants,
        ).await;
    }

    StorageEngine::delete_bucket_physical(&ws_id.to_string(), &bucket.slug, &bucket.access_type).await?;

    // Remove any cron jobs targeting this bucket.
    let _ = sqlx::query!("DELETE FROM cron_jobs WHERE target_type = 'storage' AND target_id = $1", bucket_id)
        .execute(&state.pool).await;

    sqlx::query!("DELETE FROM storage_buckets WHERE id = $1", bucket_id)
        .execute(&state.pool)
        .await?;

    // Remove the bucket's published project-pool var and reload any linked apps.
    let linked = crate::utils::app_env::unpublish_project_env(&state.pool, "storage", bucket_id).await;
    for inst in linked {
        crate::controllers::env_variable_controller::hot_reload_if_running(&state.pool, inst);
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_objects(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(bucket_slug): Path<String>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<Paginated<ObjectResponse>>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let bucket = sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE workspace_id = $1 AND slug = $2"
    )
    .bind(ws_id)
    .bind(&bucket_slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Bucket '{}' not found.", bucket_slug)))?;

    let (page, page_size, offset) = pagination.resolve();

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_objects WHERE bucket_id = $1")
        .bind(bucket.id)
        .fetch_one(&state.pool)
        .await?;

    let objects = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE bucket_id = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
    )
    .bind(bucket.id)
    .bind(page_size)
    .bind(offset)
    .fetch_all(&state.pool)
    .await?;

    let items = objects
        .into_iter()
        .map(|o| {
            let virtual_url = calculate_virtual_url(o.id, &o.file_path, &bucket.slug, ws_id, &bucket.access_type);
            ObjectResponse {
                id: o.id,
                bucket_id: o.bucket_id,
                file_path: o.file_path,
                size_bytes: o.size_bytes,
                mime_type: o.mime_type,
                etag: o.etag,
                status: o.status,
                processing_stage: o.processing_stage,
                compression: o.compression,
                original_size_bytes: o.original_size_bytes,
                is_optimized: o.is_optimized,
                image_dimensions: o.image_dimensions,
                has_variants: o.meta_data.0.has_variants,
                variants: o.meta_data.0.variants,
                virtual_url,
                created_at: o.created_at,
            }
        })
        .collect();

    Ok(Json(Paginated::new(items, total, page, page_size)))
}

pub async fn delete_object(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(object_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE id = $1"
    )
    .bind(object_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("File not found.".to_string()))?;

    let auth = authenticate_request(
        &state.pool,
        &headers,
        None,
        None,
        None,
        Some(object.bucket_id),
    ).await?;

    let (ws_id, bucket_slug, access_type) = match auth {
        StorageAuth::PlatformUser { claims, ws_id } => {
            check_user_permission(&state.pool, &claims, ws_id, "volume:delete").await?;

            let (bucket_ws_id, bucket_slug, access_type): (Uuid, String, BucketAccessType) = sqlx::query_as(
                "SELECT workspace_id, slug, access_type FROM storage_buckets WHERE id = $1"
            )
            .bind(object.bucket_id)
            .fetch_one(&state.pool)
            .await?;

            if bucket_ws_id != ws_id {
                return Err(AppError::Permission("You do not have permission to delete this file.".to_string()));
            }

            (ws_id, bucket_slug, access_type)
        }
        StorageAuth::BucketCredentials { bucket, ws_id } => {
            (ws_id, bucket.slug, bucket.access_type)
        }
    };

    StorageEngine::delete_object_physical(
        &ws_id.to_string(),
        &bucket_slug,
        &access_type,
        &object.file_path,
        object.compression,
        &object.meta_data.0.variants,
    ).await?;

    sqlx::query!("DELETE FROM storage_objects WHERE id = $1", object_id)
        .execute(&state.pool)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn serve_public_file(
    State(state): State<AppState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Path((workspace_id, bucket_slug, file_path)): Path<(Uuid, String, String)>,
) -> Result<axum::response::Response, AppError> {
    let bucket_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM storage_buckets WHERE workspace_id = $1 AND slug = $2"
    )
    .bind(workspace_id)
    .bind(&bucket_slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Storage bucket not found.".to_string()))?;

    // All buckets are private. Access is granted via the bucket's own key pair
    // (?app_id=&secret_key=) — the vars published to the project pool that deployed
    // apps connect with.
    {
        if let (Some(prov_app), Some(prov_secret)) = (params.get("app_id"), params.get("secret_key")) {
            let creds = sqlx::query!(
                "SELECT app_id, secret_key_encrypted, secret_key_nonce FROM storage_buckets WHERE id = $1",
                bucket_id
            )
            .fetch_one(&state.pool)
            .await?;
            let ok = match (creds.app_id, creds.secret_key_encrypted, creds.secret_key_nonce) {
                (Some(a), Some(e), Some(n)) => {
                    a == *prov_app
                        && crate::utils::crypto::decrypt_env_value(&e, &n).map(|s| s == *prov_secret).unwrap_or(false)
                }
                _ => false,
            };
            if !ok {
                return Err(AppError::Permission("Invalid storage credentials (app_id/secret_key).".to_string()));
            }
        } else {
            return Err(AppError::Permission(
                "Access denied to private bucket. Provide ?app_id=&secret_key=.".to_string(),
            ));
        }
    }

    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE bucket_id = $1 AND (file_path = $2 OR EXISTS (
            SELECT 1 FROM jsonb_each(meta_data->'variants') AS v 
            WHERE v.value->>'filePath' = $2 OR v.value->>'file_path' = $2
         ))"
    )
    .bind(bucket_id)
    .bind(&file_path)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Requested file not found.".to_string()))?;

    if object.status != StorageStatus::Ready && object.status != StorageStatus::Processing {
        return Err(AppError::Validation("File is not ready.".to_string()));
    }

    let (variant_file_path, variant_mime_type, variant_size_bytes) = if object.file_path == file_path {
        (object.file_path, object.mime_type, object.size_bytes)
    } else {
        let meta = &object.meta_data.0;
        let mut found = None;
        if let Some(variants) = &meta.variants {
            for (_, var) in variants {
                if var.file_path == file_path {
                    let ext = StdPath::new(&file_path).extension().and_then(|e| e.to_str()).unwrap_or("png");
                    let mime = match ext {
                        "webp" => "image/webp".to_string(),
                        "avif" => "image/avif".to_string(),
                        "jpg" | "jpeg" => "image/jpeg".to_string(),
                        _ => object.mime_type.clone(),
                    };
                    found = Some((var.file_path.clone(), mime, var.size_bytes));
                    break;
                }
            }
        }
        found.ok_or_else(|| AppError::NotFound("Requested variant file not found.".to_string()))?
    };

    let bucket_dir = StorageEngine::get_bucket_path(&workspace_id.to_string(), &bucket_slug, &BucketAccessType::PrivateStorage);
    let full_disk_path = bucket_dir.join(&variant_file_path);

    let file = tokio::fs::File::open(&full_disk_path)
        .await
        .map_err(|e| AppError::Infrastructure(format!("Failed to open file on host disk: {}", e)))?;

    let body = Body::from_stream(tokio_util::io::ReaderStream::new(file));

    let response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, variant_mime_type)
        .header(axum::http::header::CONTENT_LENGTH, variant_size_bytes)
        .header(
            axum::http::header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{}\"", StdPath::new(&variant_file_path).file_name().unwrap_or_default().to_string_lossy()),
        )
        .body(body)
        .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to construct response: {}", e)))?;

    Ok(response)
}

pub async fn update_bucket(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(bucket_id): Path<Uuid>,
    Json(payload): Json<UpdateBucketRequest>,
) -> Result<Json<BucketResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let bucket = sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE workspace_id = $1 AND id = $2"
    )
    .bind(ws_id)
    .bind(bucket_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Storage bucket not found.".to_string()))?;

    let name = payload.name.unwrap_or(bucket.name);
    // Buckets are private-only; the access type can no longer be changed.
    let access_type = BucketAccessType::PrivateStorage;
    let is_public = payload.is_public.unwrap_or(bucket.is_public);
    let allowed_file_types = payload.allowed_file_types.or(bucket.allowed_file_types);
    let max_size = payload.max_bucket_size_bytes.unwrap_or(bucket.max_bucket_size_bytes);
    let max_file_size = payload.max_file_size_bytes.unwrap_or(bucket.max_file_size_bytes);
    let allow_custom_processing = payload.allow_custom_processing.unwrap_or(bucket.allow_custom_processing);
    let processing_rules = payload.default_processing_rules.unwrap_or(bucket.default_processing_rules.0);

    sqlx::query!(
        "UPDATE storage_buckets
         SET name = $1, access_type = $2::bucket_access_type, is_public = $3, allowed_file_types = $4, max_bucket_size_bytes = $5, max_file_size_bytes = $6, allow_custom_processing = $7, default_processing_rules = $8, updated_at = now()
         WHERE id = $9 AND workspace_id = $10",
        name, access_type as _, is_public, allowed_file_types.as_deref(), max_size, max_file_size, allow_custom_processing, sqlx::types::Json(processing_rules.clone()) as _, bucket_id, ws_id
    )
    .execute(&state.pool)
    .await?;

    let assigned_domain: Option<String> = None;

    Ok(Json(BucketResponse {
        id: bucket_id,
        name,
        slug: bucket.slug,
        access_type,
        is_public,
        assigned_domain,
        allowed_file_types,
        max_bucket_size_bytes: max_size,
        max_file_size_bytes: max_file_size,
        allow_custom_processing,
        default_processing_rules: processing_rules,
        created_at: bucket.created_at,
    }))
}

// Bucket access is via the per-bucket app_id/secret_key key pair (published to the
// project env pool, rotatable). The old workspace-JWT `?token=` mechanism was removed
// — it was a 10-year user-scoped token, redundant with and less safe than the key pair.