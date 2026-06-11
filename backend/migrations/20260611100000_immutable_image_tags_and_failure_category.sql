-- Wave 1 of the professional build pipeline:
-- 1. Immutable image tags: each build pushes its own image (tagged by build id)
--    instead of overwriting a per-instance tag. The instance records which
--    image it currently runs, enabling future rollback.
-- 2. Machine-readable failure categories alongside the human failure_reason.
ALTER TABLE app_builds ADD COLUMN IF NOT EXISTS image_tag TEXT;
ALTER TABLE app_builds ADD COLUMN IF NOT EXISTS failure_category TEXT;
ALTER TABLE app_instances ADD COLUMN IF NOT EXISTS current_image_tag TEXT;
