-- Allow a storage bucket to be associated with a project so it can publish its
-- URL into that project's env pool. Nullable: buckets remain workspace-level by
-- default. ON DELETE SET NULL keeps the bucket if its project is removed.
ALTER TABLE storage_buckets
    ADD COLUMN IF NOT EXISTS project_id UUID REFERENCES projects(id) ON DELETE SET NULL;
