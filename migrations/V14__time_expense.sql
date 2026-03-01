-- Time and expense tracking schema (Phase 2 Task 4)
-- Adds DB-backed time entries, expense entries, and billing-link fields.

CREATE TABLE IF NOT EXISTS time_entries (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    timekeeper TEXT NOT NULL,
    description TEXT NOT NULL,
    hours NUMERIC(10,2) NOT NULL CHECK (hours > 0),
    hourly_rate NUMERIC(12,2),
    entry_date DATE NOT NULL,
    billable BOOLEAN NOT NULL DEFAULT TRUE,
    billed_invoice_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_time_entries_user_matter_date
    ON time_entries(user_id, matter_id, entry_date DESC);
CREATE INDEX IF NOT EXISTS idx_time_entries_user_billed
    ON time_entries(user_id, billed_invoice_id);

CREATE TABLE IF NOT EXISTS expense_entries (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    submitted_by TEXT NOT NULL,
    description TEXT NOT NULL,
    amount NUMERIC(12,2) NOT NULL CHECK (amount > 0),
    category TEXT NOT NULL CHECK (category IN (
        'filing_fee',
        'travel',
        'postage',
        'expert',
        'copying',
        'court_reporter',
        'other'
    )),
    entry_date DATE NOT NULL,
    receipt_path TEXT,
    billable BOOLEAN NOT NULL DEFAULT TRUE,
    billed_invoice_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_expense_entries_user_matter_date
    ON expense_entries(user_id, matter_id, entry_date DESC);
CREATE INDEX IF NOT EXISTS idx_expense_entries_user_billed
    ON expense_entries(user_id, billed_invoice_id);
