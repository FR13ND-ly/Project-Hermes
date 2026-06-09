-- Migration to add external_port to app_instances table
ALTER TABLE app_instances ADD COLUMN external_port INT;
