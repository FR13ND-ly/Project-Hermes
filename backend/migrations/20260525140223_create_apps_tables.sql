CREATE TYPE app_instance_type AS ENUM ('production', 'staging', 'preview');
CREATE TYPE app_status AS ENUM ('building', 'running', 'stopped', 'failed', 'crashed');

CREATE TABLE apps (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID NOT NULL,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) NOT NULL,
    git_repository TEXT NOT NULL,
    build_command TEXT,
    start_command TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_workspace_app_slug UNIQUE (workspace_id, slug)
);

CREATE TABLE app_instances (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    branch_name VARCHAR(255) NOT NULL DEFAULT 'main',
    instance_type app_instance_type NOT NULL DEFAULT 'production',
    status app_status NOT NULL DEFAULT 'stopped',
    
    internal_port INT NOT NULL DEFAULT 3000,
    assigned_domain VARCHAR(255),
    container_name VARCHAR(255) NOT NULL UNIQUE,
    
    cpu_limit INT DEFAULT 0,
    memory_limit_mb BIGINT DEFAULT 0,
    
    meta_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_app_branch UNIQUE (app_id, branch_name)
);

CREATE INDEX idx_apps_lookup ON apps(workspace_id, project_id);
CREATE INDEX idx_app_instances_lookup ON app_instances(app_id, branch_name);