CREATE TABLE serverless_functions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    code TEXT NOT NULL,
    method VARCHAR(50) NOT NULL DEFAULT 'GET',
    route_path VARCHAR(255) NOT NULL,
    memory_limit_mb INT NOT NULL DEFAULT 128,
    env_variables JSONB NOT NULL DEFAULT '[]'::jsonb,
    status VARCHAR(50) NOT NULL DEFAULT 'draft',
    assigned_domain VARCHAR(255),
    build_logs TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_route_path_per_project UNIQUE (project_id, route_path)
);

CREATE INDEX idx_serverless_functions_lookup ON serverless_functions(workspace_id, project_id);
