use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use image::io::Reader as ImageReader;
use crate::utils::error::AppError;
use crate::models::storage_model::{CompressionType, ImageVariant, BucketAccessType, ImageProcessingOptions, ImageFormatTarget};

const BASE_STORAGE_DIR: &str = "/var/www/hermes/storage";
const SECURE_STORAGE_DIR: &str = "/var/www/hermes/secure_storage";

pub struct StorageEngine;

impl StorageEngine {
    pub fn save_image_with_options(
        img: &image::DynamicImage,
        path: &Path,
        quality: u8,
    ) -> Result<(), AppError> {
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext == "jpg" || ext == "jpeg" {
            use image::ImageEncoder;
            let file = File::create(path)
                .map_err(|e| AppError::Infrastructure(format!("Failed to create JPEG image file: {}", e)))?;
            let mut writer = BufWriter::new(file);
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality);
            encoder.write_image(
                img.as_bytes(),
                img.width(),
                img.height(),
                img.color(),
            )
            .map_err(|e| AppError::Infrastructure(format!("Failed to encode JPEG image with quality {}: {}", quality, e)))?;
        } else if ext == "webp" {
            let config = zenwebp::LossyConfig::new()
                .with_quality(quality as f32)
                .with_method(4);
            let rgba = img.to_rgba8();
            let webp_data = zenwebp::EncodeRequest::lossy(&config, rgba.as_raw(), zenwebp::PixelLayout::Rgba8, img.width(), img.height())
                .encode()
                .map_err(|e| AppError::Infrastructure(format!("Failed to encode WebP image with quality {}: {:?}", quality, e)))?;
            fs::write(path, &*webp_data)
                .map_err(|e| AppError::Infrastructure(format!("Failed to write WebP image file: {}", e)))?;
        } else {
            img.save(path)
                .map_err(|e| AppError::Infrastructure(format!("Failed to save image: {}", e)))?;
        }

        Ok(())
    }

    pub fn get_bucket_path(workspace_id: &str, bucket_slug: &str, access_type: &BucketAccessType) -> PathBuf {
        let base_dir = match access_type {
            BucketAccessType::PrivateStorage => SECURE_STORAGE_DIR,
            _ => BASE_STORAGE_DIR,
        };

        #[cfg(unix)]
        {
            Path::new(base_dir).join(workspace_id).join(bucket_slug)
        }
        #[cfg(not(unix))]
        {
            Path::new("temp_storage").join(base_dir.replace("/var/www/hermes/", "")).join(workspace_id).join(bucket_slug)
        }
    }

    pub fn save_raw_file(
        workspace_id: &str,
        bucket_slug: &str,
        access_type: &BucketAccessType,
        file_path: &str,
        data: &[u8],
    ) -> Result<PathBuf, AppError> {
        let bucket_dir = Self::get_bucket_path(workspace_id, bucket_slug, access_type);
        let final_path = bucket_dir.join(file_path);

        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| AppError::Infrastructure(format!("Failed to create folder structure: {}", e)))?;
        }

        fs::write(&final_path, data)
            .map_err(|e| AppError::Infrastructure(format!("Failed to write binary file to storage: {}", e)))?;

        Ok(final_path)
    }

    pub fn compress_file(file_path: &Path, mode: CompressionType) -> Result<i64, AppError> {
        let source_file = File::open(file_path)
            .map_err(|e| AppError::Infrastructure(format!("Failed to open original file for compression: {}", e)))?;
        let mut reader = BufReader::new(source_file);

        match mode {
            CompressionType::Brotli => {
                let compressed_path = format!("{}.br", file_path.to_string_lossy());
                let dest_file = File::create(&compressed_path)?;
                let writer = BufWriter::new(dest_file);
                let mut compressor = brotli::CompressorWriter::new(writer, 4096, 6, 22);

                std::io::copy(&mut reader, &mut compressor)?;
                let meta = fs::metadata(compressed_path)?;
                Ok(meta.len() as i64)
            }
            CompressionType::Gzip => {
                let compressed_path = format!("{}.gz", file_path.to_string_lossy());
                let dest_file = File::create(&compressed_path)?;
                let writer = BufWriter::new(dest_file);
                let mut compressor = flate2::write::GzEncoder::new(writer, flate2::Compression::default());

                std::io::copy(&mut reader, &mut compressor)?;
                let meta = fs::metadata(compressed_path)?;
                Ok(meta.len() as i64)
            }
            CompressionType::None => {
                let meta = fs::metadata(file_path)?;
                Ok(meta.len() as i64)
            }
        }
    }

    pub fn generate_image_variants_smart(
        workspace_id: &str,
        bucket_slug: &str,
        access_type: &BucketAccessType,
        relative_file_path: &str,
        options: &ImageProcessingOptions,
        stage_tx: Option<&tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<(String, HashMap<String, ImageVariant>), AppError> {
        let bucket_dir = Self::get_bucket_path(workspace_id, bucket_slug, access_type);
        let full_path = bucket_dir.join(relative_file_path);

        let img = ImageReader::open(&full_path)
            .map_err(|e| AppError::Validation(format!("Invalid image file format: {}", e)))?
            .decode()
            .map_err(|e| AppError::Fatal(anyhow::anyhow!("Image decoding engine crashed: {}", e)))?;

        let original_dimensions = format!("{}x{}", img.width(), img.height());
        let mut variants = HashMap::new();
        let file_stem = full_path.file_stem().unwrap().to_string_lossy();

        for spec in &options.variants {
            if spec.name.trim().is_empty() || spec.max_width == 0 {
                continue;
            }

            // Report the live processing stage (best-effort, drained into the DB by the caller).
            if let Some(tx) = stage_tx {
                let _ = tx.send(format!("variant:{}", spec.name));
            }

            // Never upscale: clamp the target width to the source width.
            let target_width = std::cmp::min(spec.max_width, img.width());
            let mut scaled = img.resize(target_width, img.height(), image::imageops::FilterType::Lanczos3);
            if options.force_square {
                let min_dim = std::cmp::min(scaled.width(), scaled.height());
                scaled = scaled.crop_imm(0, 0, min_dim, min_dim);
            }

            let ext = match spec.format {
                ImageFormatTarget::Webp => "webp",
                ImageFormatTarget::Avif => "avif",
                ImageFormatTarget::Jpg => "jpg",
                ImageFormatTarget::Original => full_path.extension().and_then(|e| e.to_str()).unwrap_or("png"),
            };

            // Sanitize the user-supplied name for the on-disk filename; the map key
            // keeps the original name for display.
            let safe_name: String = spec.name.chars()
                .map(|c| if c.is_alphanumeric() { c } else { '_' })
                .collect();

            let variant_relative_path = if let Some(p_rel) = Path::new(relative_file_path).parent() {
                if p_rel.as_os_str().is_empty() {
                    format!("{}_{}.{}", file_stem, safe_name, ext)
                } else {
                    format!("{}/{}_{}.{}", p_rel.to_string_lossy(), file_stem, safe_name, ext)
                }
            } else {
                format!("{}_{}.{}", file_stem, safe_name, ext)
            };

            let variant_full_path = bucket_dir.join(&variant_relative_path);
            Self::save_image_with_options(&scaled, &variant_full_path, options.quality)?;

            let file_size = fs::metadata(&variant_full_path)?.len() as i64;
            variants.insert(
                spec.name.clone(),
                ImageVariant {
                    file_path: variant_relative_path,
                    size_bytes: file_size,
                    dimensions: format!("{}x{}", scaled.width(), scaled.height()),
                },
            );
        }

        Ok((original_dimensions, variants))
    }

    pub async fn upload_to_s3_if_enabled(
        workspace_id: &str,
        bucket_slug: &str,
        _access_type: &BucketAccessType,
        relative_file_path: &str,
        local_path: &Path,
    ) -> Result<(), AppError> {
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

            let file_data = fs::read(local_path)
                .map_err(|e| AppError::Infrastructure(format!("Failed to read local file to sync to S3: {}", e)))?;

            let s3_path = format!("hermes/{}/{}/{}", workspace_id, bucket_slug, relative_file_path);

            bucket.put_object(&s3_path, &file_data)
                .await
                .map_err(|e| AppError::Infrastructure(format!("Failed to upload file to S3: {}", e)))?;
        }
        Ok(())
    }

    pub async fn sync_object_to_s3_and_cleanup(
        workspace_id: &str,
        bucket_slug: &str,
        access_type: &BucketAccessType,
        relative_path: &str,
        compression: CompressionType,
        variants: &Option<HashMap<String, ImageVariant>>,
    ) -> Result<(), AppError> {
        let provider = std::env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "local".to_string());
        if provider != "s3" {
            return Ok(());
        }

        let bucket_dir = Self::get_bucket_path(workspace_id, bucket_slug, access_type);

        // 1. Upload original file
        let original_local = bucket_dir.join(relative_path);
        Self::upload_to_s3_if_enabled(workspace_id, bucket_slug, access_type, relative_path, &original_local).await?;

        // 2. Upload compressed file if any
        match compression {
            CompressionType::Brotli => {
                let rel_comp = format!("{}.br", relative_path);
                let local_comp = bucket_dir.join(&rel_comp);
                Self::upload_to_s3_if_enabled(workspace_id, bucket_slug, access_type, &rel_comp, &local_comp).await?;
                let _ = fs::remove_file(local_comp);
            }
            CompressionType::Gzip => {
                let rel_comp = format!("{}.gz", relative_path);
                let local_comp = bucket_dir.join(&rel_comp);
                Self::upload_to_s3_if_enabled(workspace_id, bucket_slug, access_type, &rel_comp, &local_comp).await?;
                let _ = fs::remove_file(local_comp);
            }
            CompressionType::None => {}
        }

        // 3. Upload image variants if any
        if let Some(vars) = variants {
            for (_, var) in vars {
                let local_var = bucket_dir.join(&var.file_path);
                Self::upload_to_s3_if_enabled(workspace_id, bucket_slug, access_type, &var.file_path, &local_var).await?;
                let _ = fs::remove_file(local_var);
            }
        }

        // 4. Remove original local file
        let _ = fs::remove_file(original_local);

        Ok(())
    }

    pub async fn delete_object_physical(
        workspace_id: &str,
        bucket_slug: &str,
        access_type: &BucketAccessType,
        relative_path: &str,
        compression: CompressionType,
        variants: &Option<HashMap<String, ImageVariant>>,
    ) -> Result<(), AppError> {
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

            // 1. Delete original
            let s3_path = format!("hermes/{}/{}/{}", workspace_id, bucket_slug, relative_path);
            let _ = bucket.delete_object(&s3_path).await;

            // 2. Delete compressed
            match compression {
                CompressionType::Brotli => {
                    let _ = bucket.delete_object(&format!("{}.br", s3_path)).await;
                }
                CompressionType::Gzip => {
                    let _ = bucket.delete_object(&format!("{}.gz", s3_path)).await;
                }
                CompressionType::None => {}
            }

            // 3. Delete variants
            if let Some(vars) = variants {
                for (_, var) in vars {
                    let var_s3_path = format!("hermes/{}/{}/{}", workspace_id, bucket_slug, var.file_path);
                    let _ = bucket.delete_object(&var_s3_path).await;
                }
            }
        } else {
            let bucket_dir = Self::get_bucket_path(workspace_id, bucket_slug, access_type);
            
            // 1. Delete original
            let original_path = bucket_dir.join(relative_path);
            let _ = fs::remove_file(&original_path);

            // 2. Delete compressed
            match compression {
                CompressionType::Brotli => {
                    let _ = fs::remove_file(bucket_dir.join(format!("{}.br", relative_path)));
                }
                CompressionType::Gzip => {
                    let _ = fs::remove_file(bucket_dir.join(format!("{}.gz", relative_path)));
                }
                CompressionType::None => {}
            }

            // 3. Delete variants
            if let Some(vars) = variants {
                for (_, var) in vars {
                    let _ = fs::remove_file(bucket_dir.join(&var.file_path));
                }
            }
        }

        Ok(())
    }

    pub async fn delete_bucket_physical(
        workspace_id: &str,
        bucket_slug: &str,
        access_type: &BucketAccessType,
    ) -> Result<(), AppError> {
        let provider = std::env::var("STORAGE_PROVIDER").unwrap_or_else(|_| "local".to_string());
        
        if provider != "s3" {
            let bucket_dir = Self::get_bucket_path(workspace_id, bucket_slug, access_type);
            if bucket_dir.exists() {
                let _ = fs::remove_dir_all(&bucket_dir);
            }
        }
        Ok(())
    }
}