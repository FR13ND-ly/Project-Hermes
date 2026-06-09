CREATE TYPE domain_routing_type AS ENUM ('reverse_proxy', 'static_host', 'custom');
CREATE TYPE domain_status AS ENUM ('pending_verification', 'active', 'failed');

CREATE TABLE domains (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    fqdn VARCHAR(255) NOT NULL,
    
    routing_type domain_routing_type NOT NULL DEFAULT 'reverse_proxy',
    client_max_body_size INTEGER NOT NULL DEFAULT 50,
    is_ssl BOOLEAN NOT NULL DEFAULT true,
    status domain_status NOT NULL DEFAULT 'pending_verification',
    
    nginx_target_host VARCHAR(255),
    nginx_root_path VARCHAR(255),
    nginx_config_content TEXT,
    
    cloudflare_zone_id VARCHAR(100),
    cloudflare_record_id VARCHAR(100),
    cf_proxy_active BOOLEAN NOT NULL DEFAULT true,
    
    created_by UUID NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    
    UNIQUE(workspace_id, fqdn)
);

CREATE INDEX idx_domains_fqdn ON domains(fqdn);