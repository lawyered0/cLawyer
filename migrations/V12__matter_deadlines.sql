-- Matter deadlines core schema (Phase 2 Task 2)
-- Adds DB-backed deadline tracking for legal matters.

CREATE TABLE IF NOT EXISTS matter_deadlines (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    title TEXT NOT NULL,
    deadline_type TEXT NOT NULL CHECK (deadline_type IN (
        'court_date',
        'filing',
        'statute_of_limitations',
        'response_due',
        'discovery_cutoff',
        'internal'
    )),
    due_at TIMESTAMPTZ NOT NULL,
    completed_at TIMESTAMPTZ,
    reminder_days JSONB NOT NULL DEFAULT '[]'::jsonb,
    rule_ref TEXT,
    computed_from UUID REFERENCES matter_deadlines(id) ON DELETE SET NULL,
    task_id UUID REFERENCES matter_tasks(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_matter_deadlines_user_matter_due
    ON matter_deadlines(user_id, matter_id, due_at);
CREATE INDEX IF NOT EXISTS idx_matter_deadlines_user_matter_completed
    ON matter_deadlines(user_id, matter_id, completed_at);
CREATE INDEX IF NOT EXISTS idx_matter_deadlines_rule_ref
    ON matter_deadlines(rule_ref);
