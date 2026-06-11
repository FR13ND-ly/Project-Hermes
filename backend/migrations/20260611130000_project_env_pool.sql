-- Project-level environment variable pool. A shared set of vars per project that
-- apps can opt into by linking. Resources (databases, storage, ...) publish their
-- connection env here via the `source` / `source_id` columns so it can be cleaned
-- up automatically when the resource is removed.
CREATE TABLE project_env_variables (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    key VARCHAR(255) NOT NULL,
    encrypted_value TEXT NOT NULL,
    nonce VARCHAR(32) NOT NULL,
    is_secret BOOLEAN NOT NULL DEFAULT true,
    source VARCHAR(32) NOT NULL DEFAULT 'manual', -- manual | database | storage | serverless
    source_id UUID,                                -- resource that owns this var (for cleanup)
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_project_env_key UNIQUE (project_id, key)
);

CREATE INDEX idx_project_env_project ON project_env_variables(project_id);

-- Live opt-in links: which app instances reference which project env vars.
-- ON DELETE CASCADE on project_env_id means deleting a project var (or the
-- resource that owns it) propagates by detaching it from every linked app.
CREATE TABLE app_env_links (
    app_instance_id UUID NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    project_env_id UUID NOT NULL REFERENCES project_env_variables(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app_instance_id, project_env_id)
);

CREATE INDEX idx_app_env_links_instance ON app_env_links(app_instance_id);
