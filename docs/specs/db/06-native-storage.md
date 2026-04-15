# Database Spec: Native Smart Storage

**Date:** 15-04-2026
**Status:** Proposed

## 1. Domain Overview
This module provides a native, intelligent file storage BaaS (Backend-as-a-Service). Unlike dumb object stores (like S3), the Hermes Rust backend acts as a media processing pipeline, handling on-the-fly image resizing, format conversion (e.g., to WebP), and Blurhash generation before saving the binary data to a Kubernetes Persistent Volume.

## 2. Table Definitions

### 2.1 Table: `storage_buckets`
**Description:** Configuration and quota management for a project's media storage.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `project_id` | UUID | FK -> projects.id, Not Null | The parent project |
| `name` | VARCHAR(63) | Unique, Not Null | Bucket name |
| `bucket_api_key` | UUID | Default gen_random_uuid() | Key for direct BaaS client uploads |
| `file_max_size_mb` | INT | Default 10 | Max size per individual file |
| `compression_enabled`| BOOLEAN | Default false | Trigger Rust image compression pipeline |
| `img_resize_max_width`| INT | Default 1920 | Auto-downscale large images |
| `img_default_format` | VARCHAR(10) | Default 'webp' | Target format for image processing |
| `allowed_mime_types` | JSONB | Default '["*/*"]' | Array of allowed types (e.g. `["image/png"]`) |
| `quota_bytes` | BIGINT | Default 1073741824 | Total bucket limit (default 1GB) |
| `used_bytes` | BIGINT | Default 0 | Real-time tracker for fast validation |
| `is_active` | BOOLEAN | Default true | Temporarily lock uploads/downloads |
| `created_at` | TIMESTAMPTZ | Default now() | Creation timestamp |

### 2.2 Table: `storage_files`
**Description:** Represents individual files and folders, caching metadata and image processing results.

| Column | Type | Constraints | Description |
| :--- | :--- | :--- | :--- |
| `id` | UUID | PK, Default gen_random_uuid() | Unique identifier |
| `bucket_id` | UUID | FK -> storage_buckets.id | Parent bucket reference |
| `name` | VARCHAR(255)| Not Null | File or folder name |
| `logical_path` | TEXT | Not Null | Virtual path (e.g. `/images/avatars/`) |
| `disk_path` | TEXT | Not Null | Physical path on the K8s PVC |
| `size_bytes` | BIGINT | Default 0 | File size on disk |
| `mime_type` | VARCHAR(100)| Not Null | e.g. `image/webp`, `application/pdf` |
| `is_folder` | BOOLEAN | Default false | Virtual directory marker for UI |
| `blurhash` | VARCHAR(100)| | Cached Blurhash string for smooth UI loading |
| `uploaded_by` | UUID | FK -> app_users.id, Nullable| Which BaaS end-user uploaded this |
| `created_at` | TIMESTAMPTZ | Default now() | Upload timestamp |

## 3. Relationships & Constraints
* **Storage Architecture:** The Rust backend will mount a shared Kubernetes `PersistentVolumeClaim` (e.g., at `/var/hermes/storage`). The `disk_path` maps to this volume.
* **Image Processing Pipeline:** If `compression_enabled` is true and the `mime_type` is an image, the Rust API will intercept the upload, resize it using the `image` crate, convert it to `img_default_format`, calculate the `blurhash`, and *only then* write it to disk and save this record.
* **Quota Validation:** Before writing any data to disk, Rust must check if `bucket.used_bytes + new_file.size_bytes > bucket.quota_bytes`.