-- Add GitHub integration columns to users table
ALTER TABLE users ADD COLUMN github_token VARCHAR(255);
ALTER TABLE users ADD COLUMN github_username VARCHAR(100);
