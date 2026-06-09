-- Migration to shorten existing database container names to avoid K8s label limits
UPDATE databases
SET container_name = 'h-db-' || type || '-' || substring(id::text from 1 for 8)
WHERE length(container_name) > 30;
