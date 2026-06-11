-- Add automated backup configuration to databases table
ALTER TABLE databases ADD COLUMN IF NOT EXISTS backup_enabled BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE databases ADD COLUMN IF NOT EXISTS backup_count INT NOT NULL DEFAULT 7;
ALTER TABLE databases ADD COLUMN IF NOT EXISTS last_backup_at TIMESTAMP WITH TIME ZONE;
