-- Remember the last image built for a serverless function so its env can be
-- re-applied (env-only reload) without re-running the Kaniko build.
ALTER TABLE serverless_functions ADD COLUMN current_image_tag VARCHAR(255);
