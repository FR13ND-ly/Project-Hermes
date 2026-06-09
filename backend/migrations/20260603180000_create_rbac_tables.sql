-- Create permissions table
CREATE TABLE permissions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create roles table
CREATE TABLE roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(100) NOT NULL UNIQUE,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Create role_permissions table
CREATE TABLE role_permissions (
    role_id UUID NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission_id UUID NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

-- Seed permissions
INSERT INTO permissions (name, description) VALUES
    ('workspace:update', 'Update workspace settings'),
    ('workspace:delete', 'Delete a workspace'),
    ('workspace:invite', 'Invite members to workspace'),
    ('project:create', 'Create a project'),
    ('project:read', 'View projects'),
    ('project:delete', 'Delete a project'),
    ('app:create', 'Create applications'),
    ('app:read', 'View applications'),
    ('app:update', 'Update application settings'),
    ('app:delete', 'Delete applications'),
    ('app:deploy', 'Trigger builds and deploys'),
    ('app:logs', 'Stream application logs'),
    ('app:stats', 'Stream application resource stats'),
    ('db:create', 'Provision database instances'),
    ('db:read', 'View database instances'),
    ('db:delete', 'Delete database instances'),
    ('env:read', 'View environment variables (decrypted)'),
    ('env:write', 'Create, update, or delete environment variables'),
    ('volume:create', 'Create storage volumes'),
    ('volume:read', 'View storage volumes'),
    ('volume:delete', 'Delete storage volumes'),
    ('domain:create', 'Associate custom domains'),
    ('domain:read', 'View custom domains'),
    ('domain:delete', 'Delete custom domains');

-- Seed roles
INSERT INTO roles (name, description) VALUES
    ('owner', 'Workspace Owner with full administrative control'),
    ('admin', 'Workspace Administrator who can manage all projects and resources'),
    ('developer', 'Workspace Developer who can deploy and edit resources but not delete projects or workspace'),
    ('viewer', 'Workspace Viewer who has read-only access to all resources');

-- Map permissions to roles
-- 1. Owner gets everything
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p WHERE r.name = 'owner';

-- 2. Admin gets almost everything (except workspace:delete)
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p 
WHERE r.name = 'admin' AND p.name != 'workspace:delete';

-- 3. Developer gets create, read, update, deploy on resources, env:write/read, but not delete workspace, delete projects, or delete databases.
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p 
WHERE r.name = 'developer' AND p.name IN (
    'project:read', 'project:create',
    'app:create', 'app:read', 'app:update', 'app:deploy', 'app:logs', 'app:stats',
    'db:create', 'db:read',
    'env:read', 'env:write',
    'volume:create', 'volume:read',
    'domain:create', 'domain:read', 'domain:delete'
);

-- 4. Viewer gets only read actions
INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id FROM roles r CROSS JOIN permissions p 
WHERE r.name = 'viewer' AND p.name IN (
    'project:read', 'app:read', 'app:logs', 'app:stats', 'db:read', 'volume:read', 'domain:read'
);

-- Modify workspace_members to link role_id instead of enum
ALTER TABLE workspace_members ADD COLUMN role_id UUID REFERENCES roles(id);

-- Migrate existing members' roles
UPDATE workspace_members wm 
SET role_id = r.id 
FROM roles r 
WHERE r.name = wm.role::TEXT;

-- Set NOT NULL
ALTER TABLE workspace_members ALTER COLUMN role_id SET NOT NULL;

-- Remove old role column
ALTER TABLE workspace_members DROP COLUMN role;

-- Drop the old enum type since it is no longer used
DROP TYPE workspace_role;
