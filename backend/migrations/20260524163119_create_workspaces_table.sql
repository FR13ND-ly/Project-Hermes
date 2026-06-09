CREATE TYPE workspace_role AS ENUM ('owner', 'admin', 'developer', 'viewer');

CREATE TABLE workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL,
    slug VARCHAR(100) NOT NULL UNIQUE,
    max_memory_mb INTEGER NOT NULL DEFAULT 2048,
    max_storage_gb INTEGER NOT NULL DEFAULT 10,
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE workspace_members (
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role workspace_role NOT NULL DEFAULT 'developer',
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id)
);

ALTER TABLE users 
ADD CONSTRAINT fk_users_current_workspace 
FOREIGN KEY (current_workspace_id) REFERENCES workspaces(id) ON DELETE SET NULL;

CREATE INDEX idx_workspaces_slug ON workspaces(slug);