-- Storage buckets are now private-only. Convert any existing buckets of the
-- removed access types (static_website, public_assets, app_bounded) to
-- private_storage. The Postgres enum type keeps its values (dropping enum values
-- is fragile) — they simply become unused.
UPDATE storage_buckets
SET access_type = 'private_storage'
WHERE access_type IN ('static_website', 'public_assets', 'app_bounded');
