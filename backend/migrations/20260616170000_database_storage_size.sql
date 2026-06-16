-- Per-database persistent-volume size in GB (the StatefulSet volumeClaimTemplate).
-- Default 1 keeps existing behaviour; only new databases honour a custom size
-- (volumeClaimTemplates are immutable, so existing DBs are unaffected).
ALTER TABLE databases ADD COLUMN storage_size_gb INTEGER NOT NULL DEFAULT 1;
