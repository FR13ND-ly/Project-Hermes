-- Single-row lease for leader election among control-plane replicas. The holder
-- renews `expires_at`; if it dies, the lease expires and another replica takes over.
-- Only the leader runs the singleton background workers (cron, auto-sleep, health,
-- reconcile, …); the job-queue workers stay HA-safe via FOR UPDATE SKIP LOCKED.
CREATE TABLE leader_lease (
    id INTEGER PRIMARY KEY,
    holder UUID NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
