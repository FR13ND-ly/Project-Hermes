-- Migration to add git_subpath to apps table
ALTER TABLE apps ADD COLUMN git_subpath VARCHAR(255);
