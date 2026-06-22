-- User-chosen in-cluster service/DNS alias other apps use to reach this app
-- (e.g. "backend" -> http://backend:3000). NULL = use the auto-derived name
-- (hermes-app-<slug>-<branch>).
ALTER TABLE app_instances ADD COLUMN network_alias VARCHAR(255);
