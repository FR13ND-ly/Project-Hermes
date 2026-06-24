use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use crate::models::storage_model::{StorageStatus, CompressionType, ImageVariant, BucketAccessType, BucketProcessingRules};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBucketRequest {
    pub name: String,
    pub project_id: Option<Uuid>,
    pub is_public: Option<bool>,
    pub allowed_file_types: Option<Vec<String>>,
    pub max_bucket_size_bytes: Option<i64>,
    pub max_file_size_bytes: Option<i64>,
    pub allow_custom_processing: Option<bool>,
    pub default_processing_rules: Option<BucketProcessingRules>,
    /// Publish the bucket URL into the project env pool (default: true).
    #[serde(default)]
    pub publish_to_env: Option<bool>,
    /// Override the suggested env key for the published bucket URL.
    #[serde(default)]
    pub env_key: Option<String>,
    #[serde(default)]
    pub publish_app_id: Option<bool>,
    #[serde(default)]
    pub app_id_env_key: Option<String>,
    #[serde(default)]
    pub publish_secret_key: Option<bool>,
    #[serde(default)]
    pub secret_key_env_key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitUploadRequest {
    pub file_path: String,
    pub size_bytes: i64,
    pub mime_type: String,
    pub custom_processing_options: Option<BucketProcessingRules>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BucketResponse {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub access_type: BucketAccessType,
    pub is_public: bool,
    pub assigned_domain: Option<String>,
    pub allowed_file_types: Option<Vec<String>>,
    pub max_bucket_size_bytes: i64,
    pub max_file_size_bytes: i64,
    pub allow_custom_processing: bool,
    pub default_processing_rules: BucketProcessingRules,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectResponse {
    pub id: Uuid,
    pub bucket_id: Uuid,
    pub file_path: String,
    pub size_bytes: i64,
    pub mime_type: String,
    pub etag: String,
    pub status: StorageStatus,
    pub processing_stage: Option<String>,
    pub compression: CompressionType,
    pub original_size_bytes: Option<i64>,
    pub is_optimized: bool,
    pub image_dimensions: Option<String>,
    pub has_variants: bool,
    pub variants: Option<HashMap<String, ImageVariant>>,
    pub virtual_url: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitUploadResponse {
    pub file_id: Uuid,
    pub status: StorageStatus,
    pub upload_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBucketRequest {
    pub name: Option<String>,
    pub access_type: Option<BucketAccessType>,
    pub is_public: Option<bool>,
    pub allowed_file_types: Option<Vec<String>>,
    pub max_bucket_size_bytes: Option<i64>,
    pub max_file_size_bytes: Option<i64>,
    pub allow_custom_processing: Option<bool>,
    pub default_processing_rules: Option<BucketProcessingRules>,
}