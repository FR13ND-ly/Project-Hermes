-- Per-cron environment, mirroring the app-instance env model:
--   * cron_env_variables = a cron's own custom vars (encrypted, like environment_variables)
--   * cron_env_links      = live links into the project env pool (like app_env_links)
-- Both cascade-delete with the cron job, so removing a cron cleans up its env.

CREATE TABLE IF NOT EXISTS cron_env_variables (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL,
    cron_job_id UUID NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    nonce TEXT NOT NULL,
    is_secret BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (cron_job_id, key)
);

CREATE TABLE IF NOT EXISTS cron_env_links (
    cron_job_id UUID NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    project_env_id UUID NOT NULL REFERENCES project_env_variables(id) ON DELETE CASCADE,
    PRIMARY KEY (cron_job_id, project_env_id)
);

CREATE INDEX IF NOT EXISTS idx_cron_env_variables_job ON cron_env_variables(cron_job_id);
CREATE INDEX IF NOT EXISTS idx_cron_env_links_job ON cron_env_links(cron_job_id);
