-- Per-app HS256 signing secret for BaaS end-user auth (Model 1: local JWT verify).
-- Encrypted at rest (AES-GCM) like env values; generated lazily on first need and
-- published into the project env pool as HERMES_AUTH_SECRET.
ALTER TABLE apps ADD COLUMN auth_secret TEXT;
ALTER TABLE apps ADD COLUMN auth_secret_nonce VARCHAR(32);
