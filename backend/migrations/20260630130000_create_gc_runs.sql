-- Garbage-collection worker run log: one row per GC pass, surfaced in the admin
-- console (Logs → GC Worker) so operators can see what was reclaimed and when.
CREATE TABLE IF NOT EXISTS gc_runs (
    id              UUID PRIMARY KEY,
    started_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at     TIMESTAMPTZ,
    -- 'running' | 'success' | 'failed'
    status          TEXT NOT NULL DEFAULT 'running',
    images_deleted  INTEGER NOT NULL DEFAULT 0,
    builds_pruned   INTEGER NOT NULL DEFAULT 0,
    jobs_pruned     INTEGER NOT NULL DEFAULT 0,
    pods_reaped     INTEGER NOT NULL DEFAULT 0,
    -- Human-readable summary / errors (one line per phase).
    detail          TEXT,
    duration_ms     BIGINT
);

CREATE INDEX IF NOT EXISTS idx_gc_runs_started_at ON gc_runs (started_at DESC);
