CREATE TYPE storage_status AS ENUM ('pending_upload', 'ready', 'processing', 'failed');
CREATE TYPE compression_type AS ENUM ('none', 'gzip', 'brotli');
CREATE TYPE bucket_access_type AS ENUM ('static_website', 'public_assets', 'private_storage', 'app_bounded');

CREATE TABLE storage_buckets (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) NOT NULL,
    access_type bucket_access_type NOT NULL DEFAULT 'public_assets',
    is_public BOOLEAN DEFAULT false,
    allowed_file_types TEXT[],
    max_bucket_size_bytes BIGINT DEFAULT 1073741824,
    default_processing_rules JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_by UUID NOT NULL,

    CONSTRAINT unique_workspace_bucket_slug UNIQUE (workspace_id, slug)
);

CREATE TABLE storage_objects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bucket_id UUID NOT NULL REFERENCES storage_buckets(id) ON DELETE CASCADE,
    file_path TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    mime_type VARCHAR(255) NOT NULL DEFAULT 'application/octet-stream',
    etag VARCHAR(64) NOT NULL,
    status storage_status NOT NULL DEFAULT 'pending_upload',
    compression compression_type NOT NULL DEFAULT 'none',
    original_size_bytes BIGINT,
    is_optimized BOOLEAN DEFAULT false,
    image_dimensions VARCHAR(32),
    meta_data JSONB DEFAULT '{}'::jsonb,
    processing_options JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_bucket_file_path UNIQUE (bucket_id, file_path)
);

CREATE INDEX idx_storage_buckets_workspace ON storage_buckets(workspace_id);
CREATE INDEX idx_storage_objects_path ON storage_objects(bucket_id, file_path);
CREATE INDEX idx_storage_objects_meta ON storage_objects USING gin (meta_data);