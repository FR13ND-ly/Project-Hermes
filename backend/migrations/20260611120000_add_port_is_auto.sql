-- Track whether the container's internal port is auto-detected from the
-- Dockerfile's EXPOSE (true) or pinned manually by the user (false).
-- When true, a build that detects an EXPOSE port updates internal_port
-- automatically; once the user edits the port in settings it flips to false.
ALTER TABLE app_instances
    ADD COLUMN IF NOT EXISTS port_is_auto BOOLEAN NOT NULL DEFAULT true;
