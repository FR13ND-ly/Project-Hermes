-- Serverless redesign: a serverless INSTANCE is the container/deployment unit;
-- inside it the user defines multiple ROUTES (method + path + code). Domain, runtime,
-- memory and env all live at the instance level. Clean start — no data to migrate.

DROP TABLE IF EXISTS serverless_env_links CASCADE;
DROP TABLE IF EXISTS serverless_env_variables CASCADE;
DROP TABLE IF EXISTS serverless_builds CASCADE;
DROP TABLE IF EXISTS serverless_functions CASCADE;

-- The deployable unit (one Knative service per instance).
CREATE TABLE serverless_instances (
    id                   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id         UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    project_id           UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name                 VARCHAR(255) NOT NULL,
    runtime              VARCHAR(50)  NOT NULL DEFAULT 'nodejs-cjs', -- nodejs-cjs | nodejs-esm | python
    memory_limit_mb      INTEGER NOT NULL DEFAULT 128,
    status               VARCHAR(20) NOT NULL DEFAULT 'draft',       -- draft | building | active | failed
    assigned_domain      VARCHAR(255),
    external_port        INTEGER,
    current_image_tag    VARCHAR(255),
    inherit_project_envs BOOLEAN NOT NULL DEFAULT false,
    build_logs           TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT unique_instance_name_per_project UNIQUE (project_id, name)
);
CREATE INDEX idx_serverless_instances_project ON serverless_instances (workspace_id, project_id);

-- A route inside an instance: HTTP method + path + the handler code.
CREATE TABLE serverless_routes (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    instance_id UUID NOT NULL REFERENCES serverless_instances(id) ON DELETE CASCADE,
    method      VARCHAR(10) NOT NULL DEFAULT 'GET',  -- GET|POST|PUT|DELETE|PATCH|ANY
    route_path  VARCHAR(255) NOT NULL,
    code        TEXT NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT unique_route_per_instance UNIQUE (instance_id, method, route_path)
);
CREATE INDEX idx_serverless_routes_instance ON serverless_routes (instance_id);

-- Per-instance own env vars (encrypted; structured, with is_secret).
CREATE TABLE serverless_env_variables (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL,
    instance_id     UUID NOT NULL REFERENCES serverless_instances(id) ON DELETE CASCADE,
    key             VARCHAR(255) NOT NULL,
    encrypted_value TEXT NOT NULL,
    nonce           TEXT NOT NULL,
    is_secret       BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT unique_instance_env_key UNIQUE (instance_id, key)
);
CREATE INDEX idx_serverless_env_instance ON serverless_env_variables (instance_id);

-- Selective links from an instance to the project env pool (live reference).
CREATE TABLE serverless_env_links (
    instance_id    UUID NOT NULL REFERENCES serverless_instances(id) ON DELETE CASCADE,
    project_env_id UUID NOT NULL REFERENCES project_env_variables(id) ON DELETE CASCADE,
    PRIMARY KEY (instance_id, project_env_id)
);

-- Build history (Kaniko logs) per instance.
CREATE TABLE serverless_builds (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    instance_id  UUID NOT NULL REFERENCES serverless_instances(id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL,
    status       VARCHAR(20) NOT NULL DEFAULT 'building', -- building | success | failed
    logs         TEXT NOT NULL DEFAULT '',
    image_tag    VARCHAR(255),
    duration_sec INTEGER,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_serverless_builds_instance ON serverless_builds (instance_id, created_at DESC);
