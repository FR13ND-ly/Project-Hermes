CREATE TYPE db_type AS ENUM ('postgres', 'mysql', 'redis', 'mongodb');
CREATE TYPE db_status AS ENUM ('provisioning', 'running', 'stopped', 'failed');

CREATE TABLE databases (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    project_id UUID NOT NULL,
    app_instance_id UUID,
    
    name VARCHAR(255) NOT NULL,
    type db_type NOT NULL,
    version VARCHAR(50) NOT NULL DEFAULT 'latest',
    
    db_user VARCHAR(255) NOT NULL,
    db_password TEXT NOT NULL,
    db_name VARCHAR(255) NOT NULL,
    
    container_name VARCHAR(255) NOT NULL UNIQUE,
    internal_port INT NOT NULL,
    status db_status NOT NULL DEFAULT 'provisioning',
    
    cpu_limit INT DEFAULT 0,
    memory_limit_mb BIGINT DEFAULT 0,
    
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT unique_db_per_target UNIQUE (project_id, app_instance_id, type)
);

CREATE INDEX idx_databases_lookup ON databases(workspace_id, project_id, app_instance_id);