-- Add base_domain column to workspaces table
ALTER TABLE workspaces 
ADD COLUMN base_domain VARCHAR(255);
