-- Auto-sleep is now opt-in: new instances default to OFF.
-- Previously the column defaulted to true (with production rows flipped to false).
-- Existing instances keep whatever value they already have — only the default for
-- newly-created instances changes.
ALTER TABLE app_instances ALTER COLUMN auto_sleep_enabled SET DEFAULT false;
