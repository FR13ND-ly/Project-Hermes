-- Add Cloudflare and Ingress IP columns to workspaces table
ALTER TABLE workspaces 
ADD COLUMN cloudflare_api_token VARCHAR(255),
ADD COLUMN cloudflare_zone_id VARCHAR(100),
ADD COLUMN ingress_ip VARCHAR(45);
