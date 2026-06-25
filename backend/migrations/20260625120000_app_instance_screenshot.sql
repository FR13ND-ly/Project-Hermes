-- Preview screenshot of a deployed instance, captured automatically when a deploy
-- becomes healthy (Vercel-style). screenshot_path is the storage key (served via the
-- tokenized screenshot endpoint); screenshot_captured_at gates the UI placeholder.
ALTER TABLE app_instances ADD COLUMN IF NOT EXISTS screenshot_path TEXT;
ALTER TABLE app_instances ADD COLUMN IF NOT EXISTS screenshot_captured_at TIMESTAMPTZ;
