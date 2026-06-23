-- BaaS becomes a standalone project resource, decoupled from apps.
--
-- A new `baas_services` entity owns the identity namespace + roles config; the signing
-- secret continues to live in the project env pool (source='baas_auth') but keyed by the
-- service id instead of an app id. The four auth tables re-key app_id -> baas_id.
--
-- Existing per-app BaaS is migrated IN PLACE (no data loss): one baas_services row per
-- app that currently uses BaaS. The HERMES_AUTH_APP_ID / HERMES_AUTH_API_URL pool vars
-- (whose VALUES are encrypted) are republished with the new service id by a startup
-- reconcile in the backend; here we only repoint their `source_id`.

CREATE TABLE baas_services (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    slug VARCHAR(255) NOT NULL,
    auth_roles_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_baas_slug_per_project UNIQUE (project_id, slug)
);
CREATE INDEX idx_baas_services_project ON baas_services(project_id);

-- app_id -> new baas_id map for every app that uses BaaS (has a published secret, a
-- custom roles config, end-users, roles, or api keys).
CREATE TEMP TABLE _baas_map (app_id UUID PRIMARY KEY, baas_id UUID NOT NULL DEFAULT gen_random_uuid());

INSERT INTO _baas_map (app_id)
SELECT a.id FROM apps a
WHERE a.auth_roles_config <> '{}'::jsonb
   OR EXISTS (SELECT 1 FROM app_users u        WHERE u.app_id = a.id)
   OR EXISTS (SELECT 1 FROM app_user_roles r   WHERE r.app_id = a.id)
   OR EXISTS (SELECT 1 FROM app_api_keys k     WHERE k.app_id = a.id)
   OR EXISTS (SELECT 1 FROM project_env_variables p WHERE p.source = 'baas_auth' AND p.source_id = a.id);

-- One service per mapped app, inheriting its name + roles config.
INSERT INTO baas_services (id, workspace_id, project_id, name, slug, auth_roles_config)
SELECT m.baas_id, a.workspace_id, a.project_id,
       a.name || ' Auth',
       lower(regexp_replace(a.name, '[^a-zA-Z0-9]+', '-', 'g')) || '-' || left(m.baas_id::text, 8),
       a.auth_roles_config
FROM _baas_map m JOIN apps a ON a.id = m.app_id;

-- Re-key the four auth tables: add baas_id, backfill from the map, drop app_id.
ALTER TABLE app_users         ADD COLUMN baas_id UUID REFERENCES baas_services(id) ON DELETE CASCADE;
ALTER TABLE app_user_roles    ADD COLUMN baas_id UUID REFERENCES baas_services(id) ON DELETE CASCADE;
ALTER TABLE app_api_keys      ADD COLUMN baas_id UUID REFERENCES baas_services(id) ON DELETE CASCADE;
ALTER TABLE app_refresh_tokens ADD COLUMN baas_id UUID REFERENCES baas_services(id) ON DELETE CASCADE;

UPDATE app_users         u SET baas_id = m.baas_id FROM _baas_map m WHERE u.app_id = m.app_id;
UPDATE app_user_roles    r SET baas_id = m.baas_id FROM _baas_map m WHERE r.app_id = m.app_id;
UPDATE app_api_keys      k SET baas_id = m.baas_id FROM _baas_map m WHERE k.app_id = m.app_id;
UPDATE app_refresh_tokens t SET baas_id = m.baas_id FROM _baas_map m WHERE t.app_id = m.app_id;

-- Repoint the env-pool baas vars (secret/app_id/api_url) onto the new service id.
UPDATE project_env_variables p
SET source_id = m.baas_id
FROM _baas_map m
WHERE p.source = 'baas_auth' AND p.source_id = m.app_id;

-- Any rows whose app wasn't mapped (shouldn't happen) are orphans — drop them so the
-- NOT NULL + FK can be enforced.
DELETE FROM app_users         WHERE baas_id IS NULL;
DELETE FROM app_user_roles    WHERE baas_id IS NULL;
DELETE FROM app_api_keys      WHERE baas_id IS NULL;
DELETE FROM app_refresh_tokens WHERE baas_id IS NULL;

ALTER TABLE app_users          ALTER COLUMN baas_id SET NOT NULL;
ALTER TABLE app_user_roles     ALTER COLUMN baas_id SET NOT NULL;
ALTER TABLE app_api_keys       ALTER COLUMN baas_id SET NOT NULL;
ALTER TABLE app_refresh_tokens ALTER COLUMN baas_id SET NOT NULL;

-- Drop the old app_id columns + their app-scoped unique constraints; re-add the
-- uniqueness scoped to baas_id. Dropping a column also drops its FK + indexes.
ALTER TABLE app_users      DROP CONSTRAINT unique_app_user_identifier;
ALTER TABLE app_users      DROP COLUMN app_id;
ALTER TABLE app_users      ADD CONSTRAINT unique_baas_user_identifier UNIQUE (baas_id, identifier);

ALTER TABLE app_user_roles DROP CONSTRAINT unique_app_user_role;
ALTER TABLE app_user_roles DROP COLUMN app_id;
ALTER TABLE app_user_roles ADD CONSTRAINT unique_baas_user_role UNIQUE (baas_id, app_user_id, role);

ALTER TABLE app_api_keys       DROP COLUMN app_id;
ALTER TABLE app_refresh_tokens DROP COLUMN app_id;

CREATE INDEX idx_app_users_baas         ON app_users(baas_id);
CREATE INDEX idx_app_user_roles_baas    ON app_user_roles(baas_id, app_user_id);
CREATE INDEX idx_app_api_keys_baas      ON app_api_keys(baas_id);
CREATE INDEX idx_app_refresh_tokens_baas ON app_refresh_tokens(baas_id);
