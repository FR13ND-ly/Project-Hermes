CREATE TYPE env_scope AS ENUM ('production', 'staging', 'preview', 'all');

DROP TABLE IF EXISTS environment_variables CASCADE;

CREATE TABLE environment_variables (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID,
    app_instance_id UUID,   
    key VARCHAR(255) NOT NULL,
    encrypted_value TEXT NOT NULL,
    nonce VARCHAR(32) NOT NULL,
    scope env_scope NOT NULL DEFAULT 'all',
    is_secret BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_env_per_scope_target UNIQUE (workspace_id, project_id, app_instance_id, key, scope)
);

CREATE INDEX idx_environment_variables_lookup 
ON environment_variables(workspace_id, project_id, app_instance_id, key, scope);