-- Migration to add runtime and inherit_project_envs to serverless_functions table
ALTER TABLE serverless_functions 
ADD COLUMN runtime VARCHAR(50) NOT NULL DEFAULT 'nodejs-cjs',
ADD COLUMN inherit_project_envs BOOLEAN NOT NULL DEFAULT FALSE;
