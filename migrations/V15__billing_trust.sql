-- Billing and trust ledger schema (Phase 2 Task 5)
-- Adds invoices, line items, and append-only trust ledger records.

CREATE TABLE IF NOT EXISTS invoices (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    invoice_number TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('draft', 'sent', 'paid', 'void', 'write_off')),
    issued_date DATE,
    due_date DATE,
    subtotal NUMERIC(12,2) NOT NULL CHECK (subtotal >= 0),
    tax NUMERIC(12,2) NOT NULL CHECK (tax >= 0),
    total NUMERIC(12,2) NOT NULL CHECK (total >= 0),
    paid_amount NUMERIC(12,2) NOT NULL CHECK (paid_amount >= 0),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE,
    UNIQUE (user_id, invoice_number)
);

CREATE INDEX IF NOT EXISTS idx_invoices_user_matter
    ON invoices(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_invoices_user_status
    ON invoices(user_id, status);
CREATE INDEX IF NOT EXISTS idx_invoices_user_due_date
    ON invoices(user_id, due_date);

CREATE TABLE IF NOT EXISTS invoice_line_items (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    invoice_id UUID NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    description TEXT NOT NULL,
    quantity NUMERIC(10,2) NOT NULL CHECK (quantity > 0),
    unit_price NUMERIC(12,2) NOT NULL CHECK (unit_price >= 0),
    amount NUMERIC(12,2) NOT NULL CHECK (amount >= 0),
    time_entry_id UUID REFERENCES time_entries(id) ON DELETE SET NULL,
    expense_entry_id UUID REFERENCES expense_entries(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_invoice_line_items_invoice
    ON invoice_line_items(invoice_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_invoice_line_items_time
    ON invoice_line_items(time_entry_id);
CREATE INDEX IF NOT EXISTS idx_invoice_line_items_expense
    ON invoice_line_items(expense_entry_id);

CREATE TABLE IF NOT EXISTS trust_ledger (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    entry_type TEXT NOT NULL CHECK (entry_type IN ('deposit', 'withdrawal', 'invoice_payment', 'refund')),
    amount NUMERIC(12,2) NOT NULL CHECK (amount > 0),
    balance_after NUMERIC(12,2) NOT NULL CHECK (balance_after >= 0),
    description TEXT NOT NULL,
    invoice_id UUID REFERENCES invoices(id) ON DELETE SET NULL,
    recorded_by TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_trust_ledger_user_matter_created
    ON trust_ledger(user_id, matter_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_trust_ledger_user_invoice
    ON trust_ledger(user_id, invoice_id);
