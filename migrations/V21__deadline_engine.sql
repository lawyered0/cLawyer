-- Phase 3: Deadline engine hardening (V21)
--
-- Adds explanation trace, rule versioning, manual override tracking,
-- unsupported-jurisdiction sentinel, and override audit log to matter_deadlines.

ALTER TABLE matter_deadlines
    ADD COLUMN IF NOT EXISTS explanation         JSONB,
    ADD COLUMN IF NOT EXISTS rule_version        TEXT,
    ADD COLUMN IF NOT EXISTS is_manual_override  BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS override_reason     TEXT,
    ADD COLUMN IF NOT EXISTS override_by         TEXT,
    ADD COLUMN IF NOT EXISTS overridden_at       TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS is_unsupported      BOOLEAN NOT NULL DEFAULT FALSE;

-- Immutable audit trail for every manual override of a computed deadline.
CREATE TABLE IF NOT EXISTS deadline_override_audit (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    deadline_id     UUID NOT NULL REFERENCES matter_deadlines(id) ON DELETE CASCADE,
    user_id         TEXT NOT NULL,
    previous_due_at TIMESTAMPTZ NOT NULL,
    new_due_at      TIMESTAMPTZ NOT NULL,
    reason          TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_deadline_override_audit_deadline
    ON deadline_override_audit(deadline_id, created_at DESC);
