-- Migration to support UX/DX improvements: Build Logging and Database Credential Reveal

CREATE TABLE app_builds (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    app_instance_id UUID NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    status VARCHAR(50) NOT NULL, -- 'building', 'succeeded', 'failed'
    logs TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_app_builds_app ON app_builds(app_id);
CREATE INDEX idx_app_builds_instance ON app_builds(app_instance_id);

ALTER TABLE databases ADD COLUMN db_password_nonce VARCHAR(255);
