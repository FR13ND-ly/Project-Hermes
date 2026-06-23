use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "bucket_access_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum BucketAccessType {
    StaticWebsite,
    PublicAssets,
    PrivateStorage,
    AppBounded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "storage_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum StorageStatus {
    PendingUpload,
    Ready,
    Processing,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "compression_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum CompressionType {
    None,
    Gzip,
    Brotli,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ImageFormatTarget {
    Original,
    Webp,
    Avif,
    Jpg,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageVariantSpec {
    pub name: String,
    pub max_width: u32,
    pub format: ImageFormatTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageProcessingOptions {
    pub convert_to: ImageFormatTarget,
    pub quality: u8,
    /// Custom output variants (name + max width + per-variant format). Replaces
    /// the old fixed presets; `#[serde(default)]` lets legacy bucket JSON that
    /// still carries `generateVariants` deserialize without error (empty until re-saved).
    #[serde(default)]
    pub variants: Vec<ImageVariantSpec>,
    pub force_square: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextProcessingOptions {
    pub pre_compress_brotli: bool,
    pub pre_compress_gzip: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct BucketProcessingRules {
    pub image_options: Option<ImageProcessingOptions>,
    pub text_options: Option<TextProcessingOptions>,
}

// Tolerant decoding: a bucket whose stored `default_processing_rules` JSON isn't a
// valid object (e.g. a legacy `[]`, `null`, or a shape from an older schema) decodes
// to the default instead of erroring. Without this, one malformed row makes the
// whole `SELECT *`-backed bucket list fail with a 500 (`invalid length 0, expected
// struct BucketProcessingRules`). Manual impl so the fallback can't itself error.
impl<'de> Deserialize<'de> for BucketProcessingRules {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Inner {
            #[serde(default)]
            image_options: Option<ImageProcessingOptions>,
            #[serde(default)]
            text_options: Option<TextProcessingOptions>,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(match serde_json::from_value::<Inner>(value) {
            Ok(inner) => BucketProcessingRules {
                image_options: inner.image_options,
                text_options: inner.text_options,
            },
            Err(_) => BucketProcessingRules::default(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImageVariant {
    pub file_path: String,
    pub size_bytes: i64,
    pub dimensions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FileMetaData {
    pub has_variants: bool,
    pub original_extension: Option<String>,
    pub variants: Option<HashMap<String, ImageVariant>>,
    pub error_reason: Option<String>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StorageBucket {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub slug: String,
    pub access_type: BucketAccessType,
    pub is_public: bool,
    pub allowed_file_types: Option<Vec<String>>,
    pub max_bucket_size_bytes: i64,
    pub max_file_size_bytes: i64,
    pub allow_custom_processing: bool,
    pub default_processing_rules: sqlx::types::Json<BucketProcessingRules>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Uuid,
    pub app_id: Option<String>,
    pub secret_key_encrypted: Option<String>,
    pub secret_key_nonce: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // The reported 500: a bucket stored `[]` for default_processing_rules.
    #[test]
    fn processing_rules_tolerates_empty_array() {
        let r: BucketProcessingRules = serde_json::from_str("[]").unwrap();
        assert_eq!(r, BucketProcessingRules::default());
    }

    #[test]
    fn processing_rules_tolerates_null_and_garbage() {
        assert_eq!(
            serde_json::from_str::<BucketProcessingRules>("null").unwrap(),
            BucketProcessingRules::default()
        );
        assert_eq!(
            serde_json::from_str::<BucketProcessingRules>(r#"{"imageOptions":"oops"}"#).unwrap(),
            BucketProcessingRules::default()
        );
    }

    #[test]
    fn processing_rules_parses_valid_object() {
        let r: BucketProcessingRules =
            serde_json::from_str(r#"{"textOptions":{"preCompressBrotli":true,"preCompressGzip":false}}"#)
                .unwrap();
        assert!(r.image_options.is_none());
        assert_eq!(r.text_options.unwrap().pre_compress_brotli, true);
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct StorageObject {
    pub id: Uuid,
    pub bucket_id: Uuid,
    pub file_path: String,
    pub size_bytes: i64,
    pub mime_type: String,
    pub etag: String,
    pub status: StorageStatus,
    pub compression: CompressionType,
    pub original_size_bytes: Option<i64>,
    pub is_optimized: bool,
    pub image_dimensions: Option<String>,
    pub processing_stage: Option<String>,
    pub meta_data: sqlx::types::Json<FileMetaData>,
    pub processing_options: sqlx::types::Json<BucketProcessingRules>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}