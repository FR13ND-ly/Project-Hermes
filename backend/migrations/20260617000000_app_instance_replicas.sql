-- Horizontal scaling for app instances: a min/max replica range. min == max
-- means a fixed replica count; max > min enables an HPA (CPU-target autoscaling).
ALTER TABLE app_instances
    ADD COLUMN IF NOT EXISTS replicas_min INTEGER NOT NULL DEFAULT 1,
    ADD COLUMN IF NOT EXISTS replicas_max INTEGER NOT NULL DEFAULT 1;
