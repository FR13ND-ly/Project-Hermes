-- Durable background-job queue. Replaces fire-and-forget tokio::spawn for
-- build/deploy work so jobs survive a process restart: a worker that dies mid-job
-- leaves the row in 'running' with a stale heartbeat, and reclaim_stale requeues
-- it. Retries with backoff via attempts/run_after.
CREATE TABLE IF NOT EXISTS jobs (
    id           UUID PRIMARY KEY,
    kind         TEXT NOT NULL,                       -- 'build' | 'deploy'
    payload      JSONB NOT NULL DEFAULT '{}'::jsonb,
    status       TEXT NOT NULL DEFAULT 'queued',      -- queued|running|succeeded|failed
    attempts     INT  NOT NULL DEFAULT 0,
    max_attempts INT  NOT NULL DEFAULT 3,
    run_after    TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_at    TIMESTAMPTZ,                          -- heartbeat; stale => worker died
    locked_by    TEXT,
    last_error   TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Claim ordering / fast scan of runnable jobs.
CREATE INDEX IF NOT EXISTS idx_jobs_runnable ON jobs (status, run_after);
