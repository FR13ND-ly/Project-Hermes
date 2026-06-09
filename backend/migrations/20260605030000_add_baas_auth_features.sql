-- Up migrations
ALTER TABLE app_users ADD COLUMN status VARCHAR(50) NOT NULL DEFAULT 'active';
ALTER TABLE app_users ADD COLUMN last_login TIMESTAMPTZ;

ALTER TABLE apps ADD COLUMN auth_roles_config JSONB NOT NULL DEFAULT '{}'::jsonb;

CREATE TABLE app_api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    key_hash VARCHAR(255) NOT NULL,
    key_prefix VARCHAR(16) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ,
    last_used_at TIMESTAMPTZ
);

CREATE INDEX idx_app_api_keys_prefix ON app_api_keys(key_prefix);
