-- Chaos experiments: controlled, auto-reverting fault injection on app instances
-- (pod-kill, scale-down, best-effort cpu-stress). A leader-gated worker reverts rows
-- whose revert_at has passed and reclaims any left 'running' across a restart. The
-- reconcile loop skips replica convergence while an experiment is 'running' so it
-- doesn't undo a deliberate scale-down.
CREATE TABLE IF NOT EXISTS chaos_experiments (
    id                 UUID PRIMARY KEY,
    workspace_id       UUID NOT NULL,
    app_id             UUID NOT NULL,
    app_instance_id    UUID NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    kind               TEXT NOT NULL,                   -- 'pod_kill' | 'scale_down' | 'cpu_stress'
    params             JSONB NOT NULL DEFAULT '{}'::jsonb,
    status             TEXT NOT NULL DEFAULT 'running',  -- running | completed | failed | cancelled
    original_replicas  INT,                             -- replica count to restore after scale_down
    message            TEXT,
    started_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    revert_at          TIMESTAMPTZ,                     -- when to auto-revert (NULL = instantaneous)
    ended_at           TIMESTAMPTZ,
    created_by         UUID
);

-- Fast "is there an active experiment for this instance?" check (reconcile guard).
CREATE INDEX IF NOT EXISTS idx_chaos_instance_status ON chaos_experiments (app_instance_id, status);
-- Worker scan for experiments due to be reverted.
CREATE INDEX IF NOT EXISTS idx_chaos_running_revert ON chaos_experiments (status, revert_at);
