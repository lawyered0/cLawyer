-- Legal Ops Hardening (Phase 1)
-- Adds trust-account reconciliation, citation verification persistence,
-- billing rate schedules, richer trust/billing snapshots, and conflict detail fields.

ALTER TABLE IF EXISTS matter_parties
    ADD COLUMN IF NOT EXISTS role_detail TEXT;

ALTER TABLE IF EXISTS conflict_clearances
    ADD COLUMN IF NOT EXISTS reviewing_attorney TEXT;

ALTER TABLE IF EXISTS conflict_clearances
    ADD COLUMN IF NOT EXISTS report_hash TEXT;

ALTER TABLE IF EXISTS conflict_clearances
    ADD COLUMN IF NOT EXISTS signed_at TIMESTAMPTZ;

ALTER TABLE IF EXISTS matter_documents
    ADD COLUMN IF NOT EXISTS readiness_state TEXT NOT NULL DEFAULT 'draft';

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS task_code TEXT;

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS activity_code TEXT;

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS resolved_rate NUMERIC(12,2);

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS rate_source TEXT;

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS block_billing_flag BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE IF EXISTS time_entries
    ADD COLUMN IF NOT EXISTS block_billing_reason TEXT;

ALTER TABLE IF EXISTS invoice_line_items
    ADD COLUMN IF NOT EXISTS task_code TEXT;

ALTER TABLE IF EXISTS invoice_line_items
    ADD COLUMN IF NOT EXISTS activity_code TEXT;

ALTER TABLE IF EXISTS invoice_line_items
    ADD COLUMN IF NOT EXISTS timekeeper TEXT;

ALTER TABLE IF EXISTS invoice_line_items
    ADD COLUMN IF NOT EXISTS resolved_rate NUMERIC(12,2);

ALTER TABLE IF EXISTS invoice_line_items
    ADD COLUMN IF NOT EXISTS rate_source TEXT;

CREATE TABLE IF NOT EXISTS trust_accounts (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    bank_name TEXT,
    account_number_last4 TEXT,
    is_primary BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_trust_accounts_primary_user
    ON trust_accounts(user_id)
    WHERE is_primary = TRUE;

ALTER TABLE IF EXISTS trust_ledger
    ADD COLUMN IF NOT EXISTS trust_account_id UUID REFERENCES trust_accounts(id) ON DELETE SET NULL;

ALTER TABLE IF EXISTS trust_ledger
    ADD COLUMN IF NOT EXISTS entry_detail TEXT;

ALTER TABLE IF EXISTS trust_ledger
    ADD COLUMN IF NOT EXISTS delta NUMERIC(12,2) NOT NULL DEFAULT 0;

ALTER TABLE IF EXISTS trust_ledger
    ADD COLUMN IF NOT EXISTS reference_number TEXT;

ALTER TABLE IF EXISTS trust_ledger
    ADD COLUMN IF NOT EXISTS source TEXT NOT NULL DEFAULT 'manual';

CREATE INDEX IF NOT EXISTS idx_trust_ledger_account_created
    ON trust_ledger(user_id, trust_account_id, created_at DESC);

UPDATE trust_ledger
SET delta = CASE
    WHEN entry_type IN ('withdrawal', 'invoice_payment', 'refund') THEN -amount
    ELSE amount
END
WHERE delta = 0;

CREATE TABLE IF NOT EXISTS trust_statement_imports (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    trust_account_id UUID NOT NULL REFERENCES trust_accounts(id) ON DELETE CASCADE,
    statement_date DATE NOT NULL,
    starting_balance NUMERIC(12,2) NOT NULL,
    ending_balance NUMERIC(12,2) NOT NULL,
    imported_by TEXT NOT NULL,
    row_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trust_statement_imports_account
    ON trust_statement_imports(user_id, trust_account_id, statement_date DESC);

CREATE TABLE IF NOT EXISTS trust_statement_lines (
    id UUID PRIMARY KEY,
    statement_import_id UUID NOT NULL REFERENCES trust_statement_imports(id) ON DELETE CASCADE,
    entry_date DATE NOT NULL,
    description TEXT NOT NULL,
    debit NUMERIC(12,2) NOT NULL,
    credit NUMERIC(12,2) NOT NULL,
    running_balance NUMERIC(12,2) NOT NULL,
    reference_number TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trust_statement_lines_import
    ON trust_statement_lines(statement_import_id, entry_date ASC);

CREATE TABLE IF NOT EXISTS trust_reconciliations (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    trust_account_id UUID NOT NULL REFERENCES trust_accounts(id) ON DELETE CASCADE,
    statement_import_id UUID NOT NULL REFERENCES trust_statement_imports(id) ON DELETE CASCADE,
    statement_ending_balance NUMERIC(12,2) NOT NULL,
    book_balance NUMERIC(12,2) NOT NULL,
    client_balance_total NUMERIC(12,2) NOT NULL,
    exceptions_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    status TEXT NOT NULL DEFAULT 'unbalanced',
    difference NUMERIC(12,2) NOT NULL,
    signed_off_by TEXT,
    signed_off_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trust_reconciliations_account
    ON trust_reconciliations(user_id, trust_account_id, created_at DESC);

CREATE TABLE IF NOT EXISTS citation_verification_runs (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    matter_document_id UUID NOT NULL REFERENCES matter_documents(id) ON DELETE CASCADE,
    provider TEXT NOT NULL,
    document_hash TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_citation_verification_runs_document
    ON citation_verification_runs(user_id, matter_document_id, created_at DESC);

CREATE TABLE IF NOT EXISTS citation_verification_results (
    id UUID PRIMARY KEY,
    run_id UUID NOT NULL REFERENCES citation_verification_runs(id) ON DELETE CASCADE,
    citation_text TEXT NOT NULL,
    normalized_citation TEXT NOT NULL,
    status TEXT NOT NULL,
    provider_reference TEXT,
    provider_title TEXT,
    detail TEXT,
    waived_by TEXT,
    waiver_reason TEXT,
    waived_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_citation_verification_results_run
    ON citation_verification_results(run_id, created_at ASC);

CREATE TABLE IF NOT EXISTS billing_rate_schedules (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT,
    timekeeper TEXT NOT NULL,
    rate NUMERIC(12,2) NOT NULL,
    effective_start DATE NOT NULL,
    effective_end DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_billing_rate_schedules_user_matter_timekeeper
    ON billing_rate_schedules(user_id, matter_id, timekeeper, effective_start DESC);
