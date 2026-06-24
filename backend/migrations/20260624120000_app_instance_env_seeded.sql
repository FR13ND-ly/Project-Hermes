-- Dockerfile ENV auto-detection should seed an instance's env only on its FIRST
-- build, not on every rebuild (re-importing kept re-creating local vars the user had
-- removed or that were superseded by a linked project-pool var → duplicates).
ALTER TABLE app_instances ADD COLUMN IF NOT EXISTS env_seeded BOOLEAN NOT NULL DEFAULT false;

-- Existing instances have already been built and configured — mark them seeded so a
-- future rebuild doesn't re-import Dockerfile defaults over their current env.
UPDATE app_instances SET env_seeded = true;
