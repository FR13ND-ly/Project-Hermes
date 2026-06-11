-- Granular build lifecycle phase + a human-readable failure reason.
-- `status` stays coarse ('building'/'succeeded'/'failed'); `phase` tracks the
-- fine-grained stage so the UI can render a progress stepper.
ALTER TABLE app_builds ADD COLUMN IF NOT EXISTS phase TEXT NOT NULL DEFAULT 'queued';
ALTER TABLE app_builds ADD COLUMN IF NOT EXISTS failure_reason TEXT;
