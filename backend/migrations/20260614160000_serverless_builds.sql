-- Build history for serverless functions (parity with app_builds): each deploy
-- records a build row whose Kaniko logs can be streamed live and replayed later.
CREATE TABLE serverless_builds (
    id           UUID PRIMARY KEY,
    function_id  UUID NOT NULL REFERENCES serverless_functions(id) ON DELETE CASCADE,
    workspace_id UUID NOT NULL,
    status       VARCHAR(20) NOT NULL DEFAULT 'building', -- building | success | failed
    logs         TEXT NOT NULL DEFAULT '',
    image_tag    VARCHAR(255),
    duration_sec INT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_serverless_builds_fn ON serverless_builds (function_id, created_at DESC);
