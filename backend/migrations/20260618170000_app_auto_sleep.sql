-- Per-app auto-sleep control. Previously hard-coded: any non-production instance
-- idle >30 min was scaled to 0. Now it's a per-instance flag + timeout.
ALTER TABLE app_instances
    ADD COLUMN IF NOT EXISTS auto_sleep_enabled BOOLEAN NOT NULL DEFAULT true,
    ADD COLUMN IF NOT EXISTS auto_sleep_after_minutes INTEGER NOT NULL DEFAULT 30;

-- Preserve the old behaviour: production instances are never auto-slept by default.
UPDATE app_instances SET auto_sleep_enabled = false WHERE instance_type = 'production';
