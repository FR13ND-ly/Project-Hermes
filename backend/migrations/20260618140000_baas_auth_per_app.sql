-- BaaS end-user auth refactor.
--
-- Changes:
--   * Identity is now an opaque, PER-APP unique `identifier` + password — nothing
--     else. No email/username/full_name coupling. (Previously app_users were GLOBAL,
--     keyed by a single globally-unique email shared across every app.)
--   * Access + rotating refresh tokens replace the single 7-day JWT.
--   * Backend-supplied custom claims are issued per-request and persisted alongside
--     the refresh token so a refreshed access token carries the same claims.
--
-- Early-stage reset: existing end-user accounts are dropped (no ambiguous global->
-- per-app mapping is attempted).

DROP TABLE IF EXISTS app_user_roles CASCADE;
DROP TABLE IF EXISTS app_users CASCADE;

-- Per-app end users. `identifier` is whatever the app chooses as its unique handle
-- (email, username, phone, external id, …) — Hermes treats it as opaque.
CREATE TABLE app_users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    identifier VARCHAR(255) NOT NULL,
    password_hash VARCHAR(255) NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    last_login TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_app_user_identifier UNIQUE (app_id, identifier)
);
CREATE INDEX idx_app_users_app ON app_users(app_id);

-- Roles stay a join table. app_id is kept (denormalised) so existing per-app role
-- queries and the project-deletion cleanup keep working unchanged.
CREATE TABLE app_user_roles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    app_user_id UUID NOT NULL REFERENCES app_users(id) ON DELETE CASCADE,
    role VARCHAR(50) NOT NULL DEFAULT 'user',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_app_user_role UNIQUE (app_id, app_user_id, role)
);
CREATE INDEX idx_app_user_roles_lookup ON app_user_roles(app_id, app_user_id);

-- Rotating, single-use refresh tokens. We store only a SHA-256 hash of the opaque
-- token. `additional_claims` persists the backend-supplied custom claims captured at
-- issue time, so refreshing re-mints an access token carrying the same data.
CREATE TABLE app_refresh_tokens (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    app_id UUID NOT NULL REFERENCES apps(id) ON DELETE CASCADE,
    app_user_id UUID NOT NULL REFERENCES app_users(id) ON DELETE CASCADE,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    additional_claims JSONB NOT NULL DEFAULT '{}'::jsonb,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_app_refresh_tokens_user ON app_refresh_tokens(app_user_id);
