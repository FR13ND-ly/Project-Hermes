-- "No limits by default": clear any leftover per-workspace resource caps left over
-- from the old 2048MB/10GB defaults so they fall back to unlimited (0). Limits stay
-- fully opt-in per workspace from Settings. (max_cpu_millicores already defaults to 0.)
UPDATE workspaces SET max_memory_mb = 0, max_storage_gb = 0;
