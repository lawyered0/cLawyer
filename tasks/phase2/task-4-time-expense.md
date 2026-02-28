# Task 4 — Phase 2D: Time & Expense Tracking

**Base branch:** `codex/phase2c-documents`
**Work branch:** `codex/phase2d-time`
**Prerequisite:** Task 1 merged. `matters` table exists.

## Objective

Persist billable time entries and expense entries linked to matters. Expose CRUD API endpoints
and a per-matter summary. No server-side timer state is required: the browser tracks the start
time and submits computed hours on stop.

`billed_invoice_id` is NULL on creation and stamped by the billing layer (Task 5).
Do not implement billing logic here; just reserve the FK column.

## New Tables

### Postgres — new file: `migrations/V14__time_expense.sql`

```sql
CREATE TABLE IF NOT EXISTS time_entries (
    id               UUID PRIMARY KEY,
    matter_id        TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    timekeeper       TEXT NOT NULL,
    description      TEXT NOT NULL,
    hours            NUMERIC(6,2) NOT NULL CHECK (hours > 0),
    hourly_rate      NUMERIC(10,2),
    entry_date       DATE NOT NULL,
    billable         BOOL NOT NULL DEFAULT TRUE,
    billed_invoice_id UUID,      -- set by billing layer; no FK enforced here
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_time_entries_matter_id ON time_entries(matter_id);
CREATE INDEX IF NOT EXISTS idx_time_entries_timekeeper ON time_entries(timekeeper);
CREATE INDEX IF NOT EXISTS idx_time_entries_billed ON time_entries(billed_invoice_id)
    WHERE billed_invoice_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_time_entries_date ON time_entries(entry_date);

CREATE TABLE IF NOT EXISTS expense_entries (
    id                UUID PRIMARY KEY,
    matter_id         TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    submitted_by      TEXT NOT NULL,
    description       TEXT NOT NULL,
    amount            NUMERIC(10,2) NOT NULL CHECK (amount > 0),
    category          TEXT NOT NULL DEFAULT 'other'
                      CHECK (category IN
                        ('filing_fee','travel','postage','expert',
                         'copying','court_reporter','other')),
    entry_date        DATE NOT NULL,
    receipt_path      TEXT,
    billable          BOOL NOT NULL DEFAULT TRUE,
    billed_invoice_id UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_expense_entries_matter_id ON expense_entries(matter_id);
CREATE INDEX IF NOT EXISTS idx_expense_entries_billed ON expense_entries(billed_invoice_id)
    WHERE billed_invoice_id IS NULL;
```

### libSQL — append to `src/db/libsql_migrations.rs`

```sql
CREATE TABLE IF NOT EXISTS time_entries (
    id                TEXT PRIMARY KEY,
    matter_id         TEXT NOT NULL,        -- logical FK → matters.id
    timekeeper        TEXT NOT NULL,
    description       TEXT NOT NULL,
    hours             REAL NOT NULL,
    hourly_rate       REAL,
    entry_date        TEXT NOT NULL,        -- "YYYY-MM-DD"
    billable          INTEGER NOT NULL DEFAULT 1,
    billed_invoice_id TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_time_entries_matter_id ON time_entries(matter_id);
CREATE INDEX IF NOT EXISTS idx_time_entries_billed ON time_entries(billed_invoice_id);

CREATE TABLE IF NOT EXISTS expense_entries (
    id                TEXT PRIMARY KEY,
    matter_id         TEXT NOT NULL,        -- logical FK → matters.id
    submitted_by      TEXT NOT NULL,
    description       TEXT NOT NULL,
    amount            REAL NOT NULL,
    category          TEXT NOT NULL DEFAULT 'other',
    entry_date        TEXT NOT NULL,        -- "YYYY-MM-DD"
    receipt_path      TEXT,
    billable          INTEGER NOT NULL DEFAULT 1,
    billed_invoice_id TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_expense_entries_matter_id ON expense_entries(matter_id);
```

## Rust Types — add to `src/db/mod.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeEntry {
    pub id: Uuid,
    pub matter_id: String,
    pub timekeeper: String,
    pub description: String,
    pub hours: f64,
    pub hourly_rate: Option<f64>,
    pub entry_date: chrono::NaiveDate,
    pub billable: bool,
    pub billed_invoice_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseEntry {
    pub id: Uuid,
    pub matter_id: String,
    pub submitted_by: String,
    pub description: String,
    pub amount: f64,
    pub category: String,
    pub entry_date: chrono::NaiveDate,
    pub receipt_path: Option<String>,
    pub billable: bool,
    pub billed_invoice_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Summary returned by the time-summary endpoint.
#[derive(Debug, Serialize, Deserialize)]
pub struct MatterTimeSummary {
    pub matter_id: String,
    pub total_hours: f64,
    pub billable_hours: f64,
    pub unbilled_hours: f64,       // billable && billed_invoice_id IS NULL
    pub total_expenses: f64,
    pub billable_expenses: f64,
    pub unbilled_expenses: f64,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
// Time entries
async fn create_time_entry(&self, entry: &TimeEntry) -> Result<(), DatabaseError>;
async fn get_time_entry(&self, id: Uuid) -> Result<Option<TimeEntry>, DatabaseError>;
async fn update_time_entry(&self, entry: &TimeEntry) -> Result<(), DatabaseError>;
async fn delete_time_entry(&self, id: Uuid) -> Result<bool, DatabaseError>;
async fn list_time_entries_for_matter(
    &self,
    matter_id: &str,
    billable_only: bool,
    unbilled_only: bool,
) -> Result<Vec<TimeEntry>, DatabaseError>;
async fn mark_time_entries_billed(
    &self,
    ids: &[Uuid],
    invoice_id: Uuid,
) -> Result<u64, DatabaseError>;  // returns rows updated

// Expense entries
async fn create_expense_entry(&self, entry: &ExpenseEntry) -> Result<(), DatabaseError>;
async fn get_expense_entry(&self, id: Uuid) -> Result<Option<ExpenseEntry>, DatabaseError>;
async fn update_expense_entry(&self, entry: &ExpenseEntry) -> Result<(), DatabaseError>;
async fn delete_expense_entry(&self, id: Uuid) -> Result<bool, DatabaseError>;
async fn list_expense_entries_for_matter(
    &self,
    matter_id: &str,
    billable_only: bool,
    unbilled_only: bool,
) -> Result<Vec<ExpenseEntry>, DatabaseError>;
async fn mark_expense_entries_billed(
    &self,
    ids: &[Uuid],
    invoice_id: Uuid,
) -> Result<u64, DatabaseError>;

// Summary
async fn matter_time_summary(
    &self,
    matter_id: &str,
) -> Result<MatterTimeSummary, DatabaseError>;
```

Implement `matter_time_summary` as a single aggregating query in each backend, not by loading
all rows into Rust and summing. Use `SUM`, `FILTER WHERE`, and `CASE` (or subqueries for
libSQL) to compute all six numeric values in one round-trip.

## API Endpoints — add to `src/channels/web/server.rs`

| Method | Path | Handler | Notes |
|--------|------|---------|-------|
| GET | `/api/matters/{id}/time` | `time_entries_list_handler` | query: `billable_only=bool`, `unbilled_only=bool` |
| POST | `/api/matters/{id}/time` | `time_entries_create_handler` | body: `{timekeeper, description, hours, hourly_rate?, entry_date, billable}` |
| PATCH | `/api/time/{id}` | `time_entries_update_handler` | cannot update `billed_invoice_id` (set by billing) |
| DELETE | `/api/time/{id}` | `time_entries_delete_handler` | reject if `billed_invoice_id` is set |
| GET | `/api/matters/{id}/expenses` | `expense_entries_list_handler` | query: `billable_only=bool`, `unbilled_only=bool` |
| POST | `/api/matters/{id}/expenses` | `expense_entries_create_handler` | body: `{submitted_by, description, amount, category, entry_date, billable, receipt_path?}` |
| PATCH | `/api/expenses/{id}` | `expense_entries_update_handler` | cannot update `billed_invoice_id` |
| DELETE | `/api/expenses/{id}` | `expense_entries_delete_handler` | reject if `billed_invoice_id` is set |
| GET | `/api/matters/{id}/time-summary` | `matter_time_summary_handler` | returns `MatterTimeSummary` |

For DELETE handlers: return `HTTP 409 Conflict` with `{"error": "already billed"}` if the
entry has a non-null `billed_invoice_id`. Do not delete billed entries.

For PATCH handlers: if the request body attempts to set `billed_invoice_id`, ignore that field
silently (billing sets it; the user cannot).

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- Use `chrono::NaiveDate` for date-only fields; do not store as full timestamps.
- In libSQL, store `NaiveDate` as `"YYYY-MM-DD"` text.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test time_entr
cargo test expense
```
