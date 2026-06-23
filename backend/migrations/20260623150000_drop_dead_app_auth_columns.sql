-- BaaS is now a standalone resource (see 20260623140000_baas_services): the roles
-- config and signing secret moved off `apps` onto `baas_services` / the env pool.
-- These columns are no longer read or written by any code — drop them.

ALTER TABLE apps DROP COLUMN IF EXISTS auth_roles_config;
ALTER TABLE apps DROP COLUMN IF EXISTS auth_secret;
ALTER TABLE apps DROP COLUMN IF EXISTS auth_secret_nonce;
