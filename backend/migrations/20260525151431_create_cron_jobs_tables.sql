CREATE TYPE cron_status AS ENUM ('active', 'paused', 'failed');

CREATE TABLE cron_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID NOT NULL,
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    schedule VARCHAR(100) NOT NULL,
    command TEXT NOT NULL,
    status cron_status NOT NULL DEFAULT 'active',
    next_run_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE cron_job_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cron_job_id UUID NOT NULL REFERENCES cron_jobs(id) ON DELETE CASCADE,
    exit_code INT NOT NULL,
    output TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_cron_jobs_lookup ON cron_jobs(workspace_id, project_id);
CREATE INDEX idx_cron_jobs_next_run ON cron_jobs(next_run_at) WHERE status = 'active';