-- Move Cloudflare / Ingress / base-domain settings from workspace level to project level.
ALTER TABLE projects
  ADD COLUMN cloudflare_api_token VARCHAR(255),
  ADD COLUMN cloudflare_zone_id   VARCHAR(100),
  ADD COLUMN ingress_ip           VARCHAR(45),
  ADD COLUMN base_domain          VARCHAR(255);

-- Backfill: each project inherits its workspace's existing CF configuration.
UPDATE projects p SET
  cloudflare_api_token = w.cloudflare_api_token,
  cloudflare_zone_id   = w.cloudflare_zone_id,
  ingress_ip           = w.ingress_ip,
  base_domain          = w.base_domain
FROM workspaces w WHERE p.workspace_id = w.id;

ALTER TABLE workspaces
  DROP COLUMN cloudflare_api_token,
  DROP COLUMN cloudflare_zone_id,
  DROP COLUMN ingress_ip,
  DROP COLUMN base_domain;
