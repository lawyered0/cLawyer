# Task 5 — Phase 2E: Billing & Invoicing

**Base branch:** `codex/phase2d-time`
**Work branch:** `codex/phase2e-billing`
**Prerequisite:** Task 4 merged. `time_entries` and `expense_entries` tables exist.

## Objective

Generate invoices from unbilled time and expense entries. Track trust account activity
per matter. The trust ledger is append-only and balance-checked at the application layer.

## New Tables

### Postgres — new file: `migrations/V15__billing.sql`

```sql
CREATE TABLE IF NOT EXISTS invoices (
    id             UUID PRIMARY KEY,
    matter_id      TEXT NOT NULL REFERENCES matters(id),
    invoice_number TEXT NOT NULL UNIQUE,
    status         TEXT NOT NULL DEFAULT 'draft'
                   CHECK (status IN ('draft','sent','paid','void','write_off')),
    issued_date    DATE,
    due_date       DATE,
    subtotal       NUMERIC(10,2) NOT NULL DEFAULT 0,
    tax            NUMERIC(10,2) NOT NULL DEFAULT 0,
    total          NUMERIC(10,2) NOT NULL DEFAULT 0,
    paid_amount    NUMERIC(10,2) NOT NULL DEFAULT 0,
    notes          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_invoices_matter_id ON invoices(matter_id);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
CREATE INDEX IF NOT EXISTS idx_invoices_invoice_number ON invoices(invoice_number);

CREATE TABLE IF NOT EXISTS invoice_line_items (
    id                UUID PRIMARY KEY,
    invoice_id        UUID NOT NULL REFERENCES invoices(id) ON DELETE CASCADE,
    description       TEXT NOT NULL,
    quantity          NUMERIC(6,2) NOT NULL DEFAULT 1,
    unit_price        NUMERIC(10,2) NOT NULL,
    amount            NUMERIC(10,2) NOT NULL,
    time_entry_id     UUID REFERENCES time_entries(id) ON DELETE SET NULL,
    expense_entry_id  UUID REFERENCES expense_entries(id) ON DELETE SET NULL,
    sort_order        INT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_invoice_line_items_invoice_id ON invoice_line_items(invoice_id);

-- Trust ledger: append-only (no UPDATE/DELETE in application code)
CREATE TABLE IF NOT EXISTS trust_ledger (
    id            UUID PRIMARY KEY,
    matter_id     TEXT NOT NULL REFERENCES matters(id),
    entry_type    TEXT NOT NULL
                  CHECK (entry_type IN
                    ('deposit','withdrawal','invoice_payment','refund')),
    amount        NUMERIC(10,2) NOT NULL CHECK (amount > 0),
    balance_after NUMERIC(10,2) NOT NULL,
    description   TEXT NOT NULL,
    invoice_id    UUID REFERENCES invoices(id) ON DELETE SET NULL,
    recorded_by   TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
    -- No updated_at: this table is immutable after insert
);

CREATE INDEX IF NOT EXISTS idx_trust_ledger_matter_id ON trust_ledger(matter_id);
CREATE INDEX IF NOT EXISTS idx_trust_ledger_created_at ON trust_ledger(matter_id, created_at);
```

### libSQL — append to `src/db/libsql_migrations.rs`

```sql
CREATE TABLE IF NOT EXISTS invoices (
    id             TEXT PRIMARY KEY,
    matter_id      TEXT NOT NULL,   -- logical FK → matters.id
    invoice_number TEXT NOT NULL UNIQUE,
    status         TEXT NOT NULL DEFAULT 'draft',
    issued_date    TEXT,            -- "YYYY-MM-DD"
    due_date       TEXT,
    subtotal       REAL NOT NULL DEFAULT 0,
    tax            REAL NOT NULL DEFAULT 0,
    total          REAL NOT NULL DEFAULT 0,
    paid_amount    REAL NOT NULL DEFAULT 0,
    notes          TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_invoices_matter_id ON invoices(matter_id);
CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);

CREATE TABLE IF NOT EXISTS invoice_line_items (
    id               TEXT PRIMARY KEY,
    invoice_id       TEXT NOT NULL,   -- logical FK → invoices.id
    description      TEXT NOT NULL,
    quantity         REAL NOT NULL DEFAULT 1,
    unit_price       REAL NOT NULL,
    amount           REAL NOT NULL,
    time_entry_id    TEXT,
    expense_entry_id TEXT,
    sort_order       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_invoice_line_items_invoice_id ON invoice_line_items(invoice_id);

CREATE TABLE IF NOT EXISTS trust_ledger (
    id            TEXT PRIMARY KEY,
    matter_id     TEXT NOT NULL,   -- logical FK → matters.id
    entry_type    TEXT NOT NULL,
    amount        REAL NOT NULL,
    balance_after REAL NOT NULL,
    description   TEXT NOT NULL,
    invoice_id    TEXT,
    recorded_by   TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_trust_ledger_matter_id ON trust_ledger(matter_id);
```

## Rust Types — add to `src/db/mod.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub matter_id: String,
    pub invoice_number: String,
    pub status: String,
    pub issued_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    pub subtotal: f64,
    pub tax: f64,
    pub total: f64,
    pub paid_amount: f64,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLineItem {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub description: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub amount: f64,
    pub time_entry_id: Option<Uuid>,
    pub expense_entry_id: Option<Uuid>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustLedgerEntry {
    pub id: Uuid,
    pub matter_id: String,
    pub entry_type: String,   // "deposit"|"withdrawal"|"invoice_payment"|"refund"
    pub amount: f64,
    pub balance_after: f64,
    pub description: String,
    pub invoice_id: Option<Uuid>,
    pub recorded_by: String,
    pub created_at: DateTime<Utc>,
}

/// Aggregated billing status for a matter.
#[derive(Debug, Serialize, Deserialize)]
pub struct MatterBillingSummary {
    pub matter_id: String,
    pub unbilled_hours: f64,
    pub unbilled_amount: f64,       // hours × rate
    pub unbilled_expenses: f64,
    pub outstanding_invoices: i64,  // sent but unpaid
    pub outstanding_amount: f64,
    pub trust_balance: f64,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
// Invoices
async fn create_invoice(&self, invoice: &Invoice) -> Result<(), DatabaseError>;
async fn get_invoice(&self, id: Uuid) -> Result<Option<Invoice>, DatabaseError>;
async fn get_invoice_by_number(&self, number: &str) -> Result<Option<Invoice>, DatabaseError>;
async fn update_invoice(&self, invoice: &Invoice) -> Result<(), DatabaseError>;
async fn list_invoices_for_matter(
    &self,
    matter_id: &str,
    status: Option<&str>,
) -> Result<Vec<Invoice>, DatabaseError>;

// Line items
async fn create_line_item(&self, item: &InvoiceLineItem) -> Result<(), DatabaseError>;
async fn list_line_items_for_invoice(
    &self,
    invoice_id: Uuid,
) -> Result<Vec<InvoiceLineItem>, DatabaseError>;
async fn delete_line_item(&self, id: Uuid) -> Result<bool, DatabaseError>;

// Trust ledger (insert only — no update/delete methods exposed)
async fn trust_current_balance(
    &self,
    matter_id: &str,
) -> Result<f64, DatabaseError>;
async fn append_trust_entry(
    &self,
    entry: &TrustLedgerEntry,
) -> Result<(), DatabaseError>;
async fn list_trust_ledger(
    &self,
    matter_id: &str,
    limit: i64,
) -> Result<Vec<TrustLedgerEntry>, DatabaseError>;

// Billing summary
async fn matter_billing_summary(
    &self,
    matter_id: &str,
) -> Result<MatterBillingSummary, DatabaseError>;
```

`trust_current_balance` retrieves the `balance_after` of the most recent row for the matter
(ORDER BY created_at DESC LIMIT 1). Returns 0.0 if no rows exist.

`append_trust_entry` must compute `balance_after` inside the DB to avoid TOCTOU:
- Postgres: use `SELECT balance_after FROM trust_ledger WHERE matter_id = $1 ORDER BY created_at DESC LIMIT 1 FOR UPDATE` inside a transaction, then insert.
- libSQL: wrap in `BEGIN IMMEDIATE`; read max balance_after, compute new balance, insert.
Both backends must reject a withdrawal/invoice_payment if it would produce a negative balance
(return `DatabaseError::Conflict { reason: "insufficient trust balance".into() }`).

## Billing Service — new file: `src/legal/billing.rs`

This is the business logic layer. Keep DB queries in the trait implementations; put
multi-step orchestration here.

```rust
/// Draft an invoice for a matter.
/// Collects all unbilled time + expense entries for the matter (optionally filtered by
/// date range) and builds an Invoice + Vec<InvoiceLineItem> in memory. Does NOT persist
/// anything until `save_draft` is called.
pub async fn draft_invoice(
    db: Arc<dyn Database>,
    matter_id: &str,
    invoice_number: &str,       // caller supplies; validated unique in DB on save
    time_entry_ids: Option<&[Uuid]>,  // None = all unbilled
    expense_entry_ids: Option<&[Uuid]>,
    tax_rate: f64,              // 0.0 – 1.0; 0.0 for most legal invoices
    notes: Option<&str>,
) -> Result<(Invoice, Vec<InvoiceLineItem>), crate::error::WorkspaceError>

/// Persist a draft invoice and its line items in a single transaction.
/// Returns DatabaseError::Conflict if invoice_number is already taken.
pub async fn save_draft(
    db: Arc<dyn Database>,
    invoice: &Invoice,
    line_items: &[InvoiceLineItem],
) -> Result<(), crate::error::WorkspaceError>

/// Finalize (send) a draft invoice.
/// Sets status → "sent", issued_date → today, stamps billed_invoice_id on all referenced
/// time + expense entries. All in a single logical operation (multiple DB calls, but
/// must all succeed or return error — no partial finalization).
pub async fn finalize_invoice(
    db: Arc<dyn Database>,
    invoice_id: Uuid,
) -> Result<Invoice, crate::error::WorkspaceError>

/// Record a payment against an invoice.
/// `from_trust`: if true, also appends a trust_ledger withdrawal for the payment amount.
/// Returns error if payment amount exceeds (total - paid_amount) or trust balance is
/// insufficient.
pub async fn record_payment(
    db: Arc<dyn Database>,
    invoice_id: Uuid,
    amount: f64,
    from_trust: bool,
    recorded_by: &str,
) -> Result<Invoice, crate::error::WorkspaceError>
```

`finalize_invoice` steps:
1. Load invoice; verify status == "draft".
2. Update status → "sent", issued_date → today.
3. Load all line items.
4. Collect `time_entry_id`s and `expense_entry_id`s from line items.
5. Call `db.mark_time_entries_billed(ids, invoice_id)` and `db.mark_expense_entries_billed(ids, invoice_id)`.
6. Return updated invoice.

If any step fails, return the error. The caller can retry; the operations are idempotent
(mark_billed is an UPDATE WHERE billed_invoice_id IS NULL).

## API Endpoints — add to `src/channels/web/server.rs`

| Method | Path | Handler | Notes |
|--------|------|---------|-------|
| GET | `/api/matters/{id}/invoices` | `invoices_list_handler` | query: `status` |
| POST | `/api/matters/{id}/invoices/draft` | `invoices_draft_handler` | body: `{invoice_number, time_entry_ids?, expense_entry_ids?, tax_rate?, notes?}` → calls `billing::draft_invoice` + `billing::save_draft` |
| GET | `/api/invoices/{id}` | `invoices_get_handler` | returns invoice + line items |
| POST | `/api/invoices/{id}/finalize` | `invoices_finalize_handler` | calls `billing::finalize_invoice` |
| POST | `/api/invoices/{id}/void` | `invoices_void_handler` | sets status → "void"; only from "draft" or "sent" |
| POST | `/api/invoices/{id}/payment` | `invoices_payment_handler` | body: `{amount, from_trust, recorded_by}` |
| GET | `/api/matters/{id}/trust` | `trust_list_handler` | query: `limit` (default 50) |
| POST | `/api/matters/{id}/trust/deposit` | `trust_deposit_handler` | body: `{amount, description, recorded_by}` |
| GET | `/api/matters/{id}/billing-summary` | `matter_billing_summary_handler` | |

`invoices_void_handler`: update status to "void". Do not un-stamp `billed_invoice_id` on
time/expense entries — the historical record that they were billed to this invoice is retained.

## Invoice Number Generation

Do not auto-generate invoice numbers. The caller always supplies one. Validate it is:
- Non-empty
- ≤ 50 characters
- Does not already exist in the DB (return HTTP 409 with `{"error": "invoice number already exists"}`)

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- Trust ledger is append-only: no `update_trust_entry` or `delete_trust_entry` methods exist.
  Do not add them. Document this in a `// APPEND-ONLY: no update/delete methods` comment in `mod.rs`.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test billing
cargo test invoice
cargo test trust
```
