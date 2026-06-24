-- Workspace-level Cloudflare credentials (mirror git_credentials): multiple tokens
-- per workspace, each bundling token + zone + base domain (one credential = one domain).
-- A project associates one; resolve_project_cf reads token/zone from it.
CREATE TABLE cloudflare_credentials (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    label TEXT NOT NULL,
    encrypted_token TEXT NOT NULL,
    nonce TEXT NOT NULL,
    zone_id TEXT NOT NULL,
    base_domain TEXT,
    created_by UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_cloudflare_credentials_ws ON cloudflare_credentials(workspace_id);

-- A project points at a credential (null = none). Deleting the credential just
-- detaches projects (no DNS until one is re-selected).
ALTER TABLE projects
    ADD COLUMN cloudflare_credential_id UUID REFERENCES cloudflare_credentials(id) ON DELETE SET NULL;

-- The old plaintext projects.cloudflare_api_token / cloudflare_zone_id columns are
-- kept for now: resolve_project_cf falls back to them until the boot reconcile
-- (reconcile_cloudflare_credentials) migrates each into an encrypted credential.
