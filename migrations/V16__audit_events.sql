CREATE TABLE IF NOT EXISTS audit_events (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    actor TEXT NOT NULL,
    matter_id TEXT,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warn', 'critical')),
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_events_user_created
    ON audit_events(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_events_user_event_type
    ON audit_events(user_id, event_type);

CREATE INDEX IF NOT EXISTS idx_audit_events_user_matter_id
    ON audit_events(user_id, matter_id);

CREATE INDEX IF NOT EXISTS idx_audit_events_user_severity
    ON audit_events(user_id, severity);
