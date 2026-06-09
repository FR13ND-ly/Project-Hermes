-- Create project_ssh_keys table to store SSH Deploy Keys for private git repositories
CREATE TABLE project_ssh_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    host VARCHAR(100) NOT NULL, -- e.g. 'github.com', 'gitlab.company.com', 'bitbucket.org'
    encrypted_private_key TEXT NOT NULL,
    nonce VARCHAR(100) NOT NULL,
    public_key TEXT NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT now(),
    
    UNIQUE(project_id, host),
    UNIQUE(project_id, name)
);

CREATE INDEX idx_project_ssh_keys_project ON project_ssh_keys(project_id);
