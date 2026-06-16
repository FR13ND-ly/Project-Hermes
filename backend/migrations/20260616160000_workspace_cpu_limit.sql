-- Per-workspace CPU cap, in millicores (k8s `m`). 0 = unlimited (opt-in, set
-- manually), matching the max_memory_mb / max_storage_gb convention.
ALTER TABLE workspaces ADD COLUMN max_cpu_millicores INTEGER NOT NULL DEFAULT 0;
