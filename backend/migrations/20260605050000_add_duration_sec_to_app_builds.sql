-- Migration to add duration_sec to app_builds table
ALTER TABLE app_builds ADD COLUMN IF NOT EXISTS duration_sec INT;
