-- Shared, cross-replica fixed-window rate limiting (replaces the per-process in-memory
-- limiter). One row per bucket (e.g. "login:<ip>", "baas:<app>:<ip>").
CREATE TABLE rate_limit_counters (
    bucket TEXT PRIMARY KEY,
    window_start TIMESTAMPTZ NOT NULL,
    count INTEGER NOT NULL
);
