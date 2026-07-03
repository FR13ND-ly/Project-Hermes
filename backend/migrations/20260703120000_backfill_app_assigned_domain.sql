-- Backfill app_instances.assigned_domain from the domains table.
--
-- Domains attached to apps were only stored in the `domains` table; the mirror
-- into app_instances.assigned_domain (which the UI and the ingress builder read)
-- was never written for target_type='app'. Existing app domains therefore never
-- surfaced. Copy the most recent app-targeted domain onto each instance that has
-- none set. add_domain/remove_domain now keep this in sync going forward.
UPDATE app_instances ai
SET assigned_domain = d.fqdn
FROM (
    SELECT DISTINCT ON (target_id) target_id, fqdn
    FROM domains
    WHERE target_type = 'app' AND target_id IS NOT NULL
    ORDER BY target_id, created_at DESC
) d
WHERE d.target_id = ai.id
  AND (ai.assigned_domain IS NULL OR ai.assigned_domain = '');
