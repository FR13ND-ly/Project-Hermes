-- Refactor environment variables to be strictly application-instance scoped.
-- Removes workspace/project level env and the scope concept.
-- NOTE: this is a destructive reset of all existing environment variables (approved).

-- Wipe all existing env vars (clean start).
DELETE FROM environment_variables;

-- Drop the legacy multi-scope unique constraint and lookup index.
ALTER TABLE environment_variables DROP CONSTRAINT IF EXISTS unique_env_per_scope_target;
DROP INDEX IF EXISTS idx_environment_variables_lookup;

-- Drop the scope concept entirely.
ALTER TABLE environment_variables DROP COLUMN IF EXISTS scope;
DROP TYPE IF EXISTS env_scope;

-- Drop the project-level linkage; env now always belongs to an app instance.
ALTER TABLE environment_variables DROP COLUMN IF EXISTS project_id;

-- Every variable must belong to a concrete app instance.
ALTER TABLE environment_variables ALTER COLUMN app_instance_id SET NOT NULL;

-- One value per key per instance.
ALTER TABLE environment_variables
    ADD CONSTRAINT unique_env_per_instance UNIQUE (app_instance_id, key);

CREATE INDEX idx_environment_variables_instance
    ON environment_variables(app_instance_id, key);
