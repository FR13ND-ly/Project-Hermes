-- Workspace-scoped git credentials (PATs) for multi-provider imports (GitHub, GitLab, …).
-- Replaces the single per-user github_token for the import/clone/detection flow.
CREATE TABLE git_credentials (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    provider        VARCHAR(20)  NOT NULL,                 -- 'github' | 'gitlab'
    host            VARCHAR(255) NOT NULL DEFAULT 'github.com',
    label           VARCHAR(120) NOT NULL,
    username        VARCHAR(255),                          -- account login, filled on verify
    encrypted_token TEXT NOT NULL,
    nonce           TEXT NOT NULL,
    created_by      UUID NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_git_credentials_ws ON git_credentials (workspace_id);

-- Which credential an app uses to clone + detect its repo (NULL = legacy token / SSH / public).
ALTER TABLE apps ADD COLUMN git_credential_id UUID REFERENCES git_credentials(id) ON DELETE SET NULL;
