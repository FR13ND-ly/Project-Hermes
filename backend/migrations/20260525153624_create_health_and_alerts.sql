ALTER TABLE app_instances ADD COLUMN health_check_path VARCHAR(255) DEFAULT '/';

CREATE TABLE app_incident_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    app_instance_id UUID NOT NULL REFERENCES app_instances(id) ON DELETE CASCADE,
    incident_type VARCHAR(100) NOT NULL,
    message TEXT NOT NULL,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_incidents_active ON app_incident_logs(app_instance_id) WHERE resolved_at IS NULL;