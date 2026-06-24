-- Cloudflare tokens now live on workspace cloudflare_credentials (referenced via
-- projects.cloudflare_credential_id). The old per-project plaintext columns are no
-- longer read or written — drop them. (Test server: no data migration needed.)
ALTER TABLE projects DROP COLUMN IF EXISTS cloudflare_api_token;
ALTER TABLE projects DROP COLUMN IF EXISTS cloudflare_zone_id;
