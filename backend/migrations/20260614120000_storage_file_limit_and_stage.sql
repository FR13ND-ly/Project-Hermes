-- Per-file upload size limit (0 = unlimited), per-upload processing override flag,
-- and granular processing stage tracking for the storage subsystem.
ALTER TABLE storage_buckets ADD COLUMN max_file_size_bytes BIGINT NOT NULL DEFAULT 0;
ALTER TABLE storage_buckets ADD COLUMN allow_custom_processing BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE storage_objects ADD COLUMN processing_stage TEXT;
