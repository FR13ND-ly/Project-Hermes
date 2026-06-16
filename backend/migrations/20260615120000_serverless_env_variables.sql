-- Per-function environment variables with the same structure as app instances'
-- `environment_variables` (encrypted value + per-var is_secret), replacing the
-- legacy unstructured `serverless_functions.env_variables` JSON blob. Existing
-- blob data is backfilled into this table at startup (see reconcile_serverless_envs).
CREATE TABLE serverless_env_variables (
    id              UUID PRIMARY KEY,
    workspace_id    UUID NOT NULL,
    function_id     UUID NOT NULL REFERENCES serverless_functions(id) ON DELETE CASCADE,
    key             VARCHAR(255) NOT NULL,
    encrypted_value TEXT NOT NULL,
    nonce           TEXT NOT NULL,
    is_secret       BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT unique_function_env_key UNIQUE (function_id, key)
);
CREATE INDEX idx_serverless_env_function ON serverless_env_variables(function_id);
