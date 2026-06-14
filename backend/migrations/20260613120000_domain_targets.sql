-- Domains can now explicitly target an app instance, a serverless function, a
-- database, or be a free-form custom nginx site. Existing rows are 'custom'
-- (they were attached via the free-text nginx_target_host).
ALTER TABLE domains ADD COLUMN target_type VARCHAR(20) NOT NULL DEFAULT 'custom';
ALTER TABLE domains ADD COLUMN target_id UUID;

CREATE INDEX IF NOT EXISTS idx_domains_target ON domains (target_type, target_id);
