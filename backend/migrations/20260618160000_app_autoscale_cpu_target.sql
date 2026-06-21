-- Per-app autoscaling CPU target: the average CPU utilization (%) at which the HPA
-- adds replicas (was hard-coded to 80). Only takes effect when replicas_max > replicas_min.
ALTER TABLE app_instances
    ADD COLUMN IF NOT EXISTS autoscale_cpu_percent INTEGER NOT NULL DEFAULT 80;
