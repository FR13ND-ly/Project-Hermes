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
    let max_size = payload.max_bucket_size_bytes.unwrap_or(1073741824);
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

    sqlx::query!(
        "INSERT INTO storage_buckets (id, workspace_id, project_id, name, slug, access_type, is_public, max_bucket_size_bytes, max_file_size_bytes, allow_custom_processing, default_processing_rules, created_by)
         VALUES ($1, $2, $3, $4, $5, $6::bucket_access_type, $7, $8, $9, $10, $11, $12)",
        bucket_id, ws_id, payload.project_id, payload.name.trim(), slug, access_type as _, is_public, max_size, max_file_size, allow_custom_processing, sqlx::types::Json(processing_rules.clone()) as _, claims.sub
    )
    .execute(&mut *tx)
    .await?;

    let assigned_domain: Option<String> = None;
    fs::create_dir_all(&bucket_dir)
        .map_err(|e| AppError::Infrastructure(format!("Failed to create bucket directory: {}", e)))?;

    tx.commit().await?;

    // Publish the bucket's URL into the project env pool when scoped to a project.
    // Opt-out via publish_to_env=false; the key defaults to BUCKET_<SLUG>_URL but a
    // custom env_key may be supplied at creation.
    if let (Some(pid), true) = (payload.project_id, payload.publish_to_env.unwrap_or(true)) {
        let in_workspace = sqlx::query_scalar!(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = $1 AND workspace_id = $2)",
            pid, ws_id
        )
        .fetch_one(&state.pool)
        .await?
        .unwrap_or(false);
        if in_workspace {
            let bucket_url = format!("https://{}.{}", slug, base_domain);
            let default_key = format!("BUCKET_{}_URL", crate::utils::app_env::sanitize_key_fragment(&slug, "STORAGE"));
            let key = match payload.env_key.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(custom) => crate::utils::app_env::sanitize_key_fragment(custom, &default_key),
                None => default_key,
            };
            let _ = crate::utils::app_env::publish_project_env(
                &state.pool, ws_id, pid, &key, &bucket_url, false, "storage", bucket_id,
            )
            .await;
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
    AuthenticatedUser(claims): AuthenticatedUser,
    Json(payload): Json<InitUploadRequest>,
) -> Result<Json<InitUploadResponse>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let clean_path = payload.file_path.trim().trim_start_matches('/').to_string();
    let bucket_slug = clean_path.split('/').next().ok_or_else(|| {
        AppError::Validation("Invalid file path format. Must include bucket prefix.".to_string())
    })?;

    let relative_file_path = clean_path.strip_prefix(bucket_slug).unwrap_or(&clean_path).trim_start_matches('/').to_string();

    if relative_file_path.is_empty() {
        return Err(AppError::Validation("File path cannot be empty after bucket resolution.".to_string()));
    }

    let bucket = sqlx::query_as::<_, StorageBucket>(
        "SELECT * FROM storage_buckets WHERE workspace_id = $1 AND slug = $2"
    )
    .bind(ws_id)
    .bind(bucket_slug)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound(format!("Target bucket '{}' not found.", bucket_slug)))?;

    // Enforce the per-file size limit (0 = unlimited).
    if bucket.max_file_size_bytes > 0 && payload.size_bytes > bucket.max_file_size_bytes {
        return Err(AppError::Validation(format!(
            "File size {} exceeds the per-file limit of {} for this bucket.",
            format_bytes(payload.size_bytes),
            format_bytes(bucket.max_file_size_bytes)
        )));
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

pub async fn download_private_file(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(file_id): Path<Uuid>,
) -> Result<axum::response::Response, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE id = $1"
    )
    .bind(file_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("Requested file not found.".to_string()))?;

    let (bucket_ws_id, bucket_slug, access_type): (Uuid, String, BucketAccessType) = sqlx::query_as(
        "SELECT workspace_id, slug, access_type FROM storage_buckets WHERE id = $1"
    )
    .bind(object.bucket_id)
    .fetch_one(&state.pool)
    .await?;

    if bucket_ws_id != ws_id {
        return Err(AppError::Permission("You do not have permission to access this storage bucket.".to_string()));
    }

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

        let s3_path = format!("hermes/{}/{}/{}", bucket_ws_id, bucket_slug, object.file_path);
        
        let presigned_url = bucket.presign_get(&s3_path, 3600, None)
            .map_err(|e| AppError::Infrastructure(format!("Failed to generate S3 presigned URL: {}", e)))?;

        let response = axum::response::Response::builder()
            .status(StatusCode::FOUND)
            .header(axum::http::header::LOCATION, presigned_url)
            .body(Body::empty())
            .map_err(|e| AppError::Fatal(anyhow::anyhow!("Failed to construct redirect response: {}", e)))?;

        return Ok(response);
    }

    let bucket_dir = StorageEngine::get_bucket_path(&bucket_ws_id.to_string(), &bucket_slug, &access_type);
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

    let buckets = sqlx::query_as::<_, StorageBucket>(
        "SELECT id, workspace_id, name, slug, access_type, is_public, allowed_file_types, max_bucket_size_bytes, max_file_size_bytes, allow_custom_processing, default_processing_rules, created_at, updated_at, created_by FROM storage_buckets WHERE workspace_id = $1 ORDER BY created_at DESC"
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
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(object_id): Path<Uuid>,
) -> Result<StatusCode, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let object = sqlx::query_as::<_, StorageObject>(
        "SELECT * FROM storage_objects WHERE id = $1"
    )
    .bind(object_id)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(|| AppError::NotFound("File not found.".to_string()))?;

    let (bucket_ws_id, bucket_slug, access_type): (Uuid, String, BucketAccessType) = sqlx::query_as(
        "SELECT workspace_id, slug, access_type FROM storage_buckets WHERE id = $1"
    )
    .bind(object.bucket_id)
    .fetch_one(&state.pool)
    .await?;

    if bucket_ws_id != ws_id {
        return Err(AppError::Permission("You do not have permission to delete this file.".to_string()));
    }

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

    // All buckets are private — a valid token is always required.
    {
        let token = params.get("token").ok_or_else(|| {
            AppError::Permission("Access denied to private storage bucket. Missing token.".to_string())
        })?;

        let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "super_secret_key".to_string());
        let token_data = jsonwebtoken::decode::<crate::middlewares::auth_middleware::Claims>(
            token,
            &jsonwebtoken::DecodingKey::from_secret(jwt_secret.as_bytes()),
            &jsonwebtoken::Validation::default(),
        )
        .map_err(|_| AppError::Auth("Invalid or expired token".to_string()))?;

        if token_data.claims.status == crate::models::user_model::UserStatus::Suspended {
            return Err(AppError::Permission("This account has been suspended".to_string()));
        }

        if token_data.claims.current_workspace_id != Some(workspace_id) && !token_data.claims.is_super_admin {
            return Err(AppError::Permission("You do not have permission to access this workspace's assets.".to_string()));
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

pub async fn generate_bucket_token(
    State(state): State<AppState>,
    AuthenticatedUser(claims): AuthenticatedUser,
    Path(bucket_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AppError> {
    let ws_id = claims.current_workspace_id.ok_or_else(|| {
        AppError::Validation("No active workspace selected.".to_string())
    })?;

    let bucket_exists = sqlx::query!(
        "SELECT id FROM storage_buckets WHERE id = $1 AND workspace_id = $2",
        bucket_id, ws_id
    )
    .fetch_optional(&state.pool)
    .await?
    .is_some();

    if !bucket_exists {
        return Err(AppError::NotFound("Storage bucket not found.".to_string()));
    }

    let jwt_secret = std::env::var("JWT_SECRET").unwrap_or_else(|_| "super_secret_key".to_string());
    let expiration = chrono::Utc::now() + chrono::Duration::days(3650); // 10 years

    let integration_claims = crate::middlewares::auth_middleware::Claims {
        sub: claims.sub,
        username: claims.username.clone(),
        email: claims.email.clone(),
        status: claims.status,
        is_super_admin: claims.is_super_admin,
        current_workspace_id: Some(ws_id),
        exp: expiration.timestamp(),
    };

    let token = jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &integration_claims,
        &jsonwebtoken::EncodingKey::from_secret(jwt_secret.as_bytes())
    ).map_err(|e| AppError::Fatal(anyhow::anyhow!(e.to_string())))?;

    Ok(Json(serde_json::json!({
        "token": token,
        "expiresAt": expiration,
    })))
}