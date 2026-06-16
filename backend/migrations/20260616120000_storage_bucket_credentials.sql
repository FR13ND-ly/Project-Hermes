-- Per-bucket access credentials (app_id + secret_key) so apps connect to a private
-- bucket with a key pair instead of a long-lived user JWT. Both are published into
-- the project env pool on bucket creation. The secret is stored encrypted.
ALTER TABLE storage_buckets ADD COLUMN app_id VARCHAR(64);
ALTER TABLE storage_buckets ADD COLUMN secret_key_encrypted TEXT;
ALTER TABLE storage_buckets ADD COLUMN secret_key_nonce TEXT;
CREATE INDEX IF NOT EXISTS idx_storage_buckets_app_id ON storage_buckets (app_id);
