-- Add external_port column to serverless_functions
ALTER TABLE serverless_functions ADD COLUMN IF NOT EXISTS external_port INT;
