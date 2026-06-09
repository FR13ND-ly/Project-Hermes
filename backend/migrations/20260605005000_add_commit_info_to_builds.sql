-- Add commit details to build logs table
ALTER TABLE app_builds ADD COLUMN commit_message TEXT;
ALTER TABLE app_builds ADD COLUMN commit_sha VARCHAR(100);
