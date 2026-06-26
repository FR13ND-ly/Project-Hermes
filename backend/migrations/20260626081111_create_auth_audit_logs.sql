-- Create auth_audit_logs table to record login/logout/password/provision events
CREATE TABLE auth_audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    identity VARCHAR(255) NOT NULL,
    action VARCHAR(50) NOT NULL,
    client_ip VARCHAR(100),
    user_agent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Add index for fast chronological sorting
CREATE INDEX idx_auth_audit_logs_created_at ON auth_audit_logs(created_at DESC);
