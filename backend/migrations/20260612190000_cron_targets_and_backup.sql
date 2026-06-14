-- Cron jobs can target an app, a database, or a storage bucket (not just apps).
-- `is_backup` marks the managed database-backup cron (preserves file storage +
-- retention + restore). Existing rows are app-targeted.
ALTER TABLE cron_jobs ADD COLUMN target_type VARCHAR(20) NOT NULL DEFAULT 'app';
ALTER TABLE cron_jobs ADD COLUMN target_id UUID;
ALTER TABLE cron_jobs ADD COLUMN is_backup BOOLEAN NOT NULL DEFAULT false;

UPDATE cron_jobs SET target_id = app_id WHERE target_id IS NULL;

-- app_id is only meaningful for app-targeted crons now.
ALTER TABLE cron_jobs ALTER COLUMN app_id DROP NOT NULL;

CREATE INDEX IF NOT EXISTS idx_cron_jobs_target ON cron_jobs (target_type, target_id);
