-- Track consecutive health-check failures so a transient blip doesn't immediately
-- mark an instance failed + fire alerts. An incident is raised only once the count
-- crosses the threshold (see start_health_check_worker).
ALTER TABLE app_instances ADD COLUMN consecutive_health_failures INTEGER NOT NULL DEFAULT 0;
