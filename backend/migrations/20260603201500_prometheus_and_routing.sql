-- Migration to support Prometheus Metrics and TCP/UDP Routing

ALTER TABLE databases ADD COLUMN is_external BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE databases ADD COLUMN external_port INT;

ALTER TABLE apps ADD COLUMN tcp_udp_ports JSONB NOT NULL DEFAULT '[]'::jsonb;
