# Phase 2 — Legal Practice Core

Build the must-have features for a solo/small law practice on top of the existing
IronClaw + cLawyer codebase. Read `CLAUDE.md` and the existing `src/legal/`, `src/db/`,
and `src/channels/web/server.rs` before writing any code so you follow established patterns.

## Ground rules (apply to every task below)

- Every new DB table must be added to **both** `migrations/V{N}__name.sql` (Postgres) and
  `src/db/libsql_migrations.rs` (libSQL). Follow the type-mapping already used there
  (UUID→TEXT, TIMESTAMPTZ→TEXT ISO-8601, JSONB→TEXT, BOOL→INTEGER 0/1).
- Every persistence operation goes through the `Database` trait in `src/db/mod.rs` first,
  then implemented in both `src/db/postgres.rs` and `src/db/libsql_backend.rs`.
- No `.unwrap()` or `.expect()` in production code. Use `crate::` not `super::`.
- Run `cargo fmt && cargo clippy --all --all-features` before finishing each task.
  Zero warnings allowed.
- Check compilation with a single feature too: `cargo check --no-default-features --features libsql`.

---

## Task 0 — Fix conflict-graph atomicity (branch: `codex/phase2-preflight`)

Base off `codex/legal-intake-conflict-graph-schema`.

Two bugs in `src/legal/matter.rs`:

1. **Reindex is not serialized.** `reindex_conflict_graph` calls `reset_conflict_graph()`
   then seeds matters one by one in separate transactions. A concurrent conflict check
   lands in the middle of a partially-rebuilt graph. Fix: add a `tokio::sync::Mutex<()>`
   in a `static LazyLock` and hold it for the full duration of reindex (reset through last
   seed). Conflict *read* paths must not acquire this lock.

2. **Per-entry seed + alias calls are not atomic.** For each entry, `seed_matter_parties`
   and `upsert_party_aliases` are two separate transactions. A crash between them leaves a
   party with no aliases. Fix: add a new `Database` trait method `seed_conflict_entry` that
   wraps both operations in a single transaction, and use it in the reindex loop.

---

## Task 1 — Matter & Client normalization (branch: `codex/phase2a-matter-client`)

Base off Task 0's branch.

**What to build:** Four new DB tables and full CRUD for each.

- `clients` — canonical client records: name, type (individual/entity), email, phone,
  address, notes. Normalized name column with trigram index for fuzzy search.
- `matters` — one DB row per matter: client_id FK, status (intake/active/pending/closed/
  archived), stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to (array),
  custom_fields (JSON object). This replaces the workspace-file-only metadata.
- `matter_tasks` — tasks linked to a matter: title, description, status, assignee, due_at,
  blocked_by (array of task IDs).
- `matter_notes` — notes linked to a matter: author, body, pinned flag.

**Startup migration:** Add `reindex_matters_from_workspace` in `src/legal/matter.rs` that
walks existing workspace matter directories, reads each `metadata.json`, upserts a client
row (looking up by normalized name first) and a matter row. Call it from `main.rs` on
startup when legal mode is enabled.

**API:** Add CRUD endpoints for clients and matters in `src/channels/web/server.rs`, plus
task and note sub-resources under `/api/matters/{id}/tasks` and `/api/matters/{id}/notes`.
The matter-create endpoint must run the existing conflict-check gate and return HTTP 409 if
hits are found and not cleared.

---

## Task 2 — Calendar & Deadlines (branch: `codex/phase2b-deadlines`)

Base off Task 1's branch.

**What to build:**

- `matter_deadlines` DB table: matter_id, title, deadline_type (court_date/filing/
  statute_of_limitations/response_due/discovery_cutoff/internal), due_at, completed_at,
  reminder_days (int array), rule_ref (e.g. "FRCP 26(a)(1)"), computed_from (self-FK for
  derived deadlines), optional task_id FK.

- **Court-rule calculator** in `src/legal/calendar.rs`: load rules from a TOML file bundled
  via `include_str!`. Each rule has an id, citation reference, calendar or court-day offset,
  and deadline_type. Ship at least FRCP 12(a)(1), 26(a)(1), 56(c)(1), and CA CCP 412.20.
  Expose `apply_rule(rule, trigger_date) -> DateTime<Utc>` and
  `deadline_from_rule(matter_id, rule, trigger, computed_from) -> MatterDeadline`.

- **Reminder integration:** when a deadline is created with non-empty `reminder_days`, create
  one-shot `Routine` rows (using the existing routines system) that fire at
  `due_at - N days` and send a notification.

**API:** CRUD under `/api/matters/{id}/deadlines`, a `POST .../compute` endpoint that
returns a computed deadline without persisting it, and `GET /api/legal/court-rules`.

---

## Task 3 — Document Management (branch: `codex/phase2c-documents`)

Base off Task 2's branch.

**What to build:** Extend the existing `memory_documents` workspace system — don't replace it.

- `matter_documents` — links a `memory_documents` row to a matter with a display name and
  category (pleading/correspondence/contract/filing/evidence/internal).
- `document_versions` — version history for a matter document: version_number, label
  (draft/filed/executed), FK back to the content snapshot in `memory_documents`.
- `document_templates` — Tera template source + variable descriptor list (JSON).

- **Docgen engine** in `src/legal/docgen.rs` using the `tera` crate: `render_template(body,
  context)` and `build_context(matter, client, extra)`. Write rendered output to the
  workspace as a new `memory_document`, then attach it via `matter_documents`.

**API:** Replace the existing stub `matter_documents_handler` and `matter_templates_handler`
with real implementations. Add `POST /api/documents/generate` (template_id + matter_id +
extra overrides → rendered document attached to matter).

---

## Task 4 — Time & Expense Tracking (branch: `codex/phase2d-time`)

Base off Task 3's branch.

**What to build:**

- `time_entries` — matter_id, timekeeper, description, hours (positive decimal), hourly_rate
  (optional), entry_date (date only), billable flag, billed_invoice_id (NULL until invoiced).
- `expense_entries` — matter_id, submitted_by, description, amount, category
  (filing_fee/travel/postage/expert/copying/court_reporter/other), entry_date, receipt_path,
  billable flag, billed_invoice_id.

Both tables need `mark_*_billed(ids, invoice_id)` bulk-update methods on the trait for the
billing layer. Delete endpoints must reject (HTTP 409) if `billed_invoice_id` is set.

Add a `matter_time_summary` trait method that returns total/billable/unbilled hours and
expenses in a single aggregating DB query (no in-Rust summing).

**API:** CRUD under `/api/matters/{id}/time` and `/api/matters/{id}/expenses`, plus
`GET /api/matters/{id}/time-summary`.

---

## Task 5 — Billing & Invoicing (branch: `codex/phase2e-billing`)

Base off Task 4's branch.

**What to build:**

- `invoices` — matter_id, invoice_number (unique, caller-supplied), status (draft/sent/paid/
  void/write_off), issued_date, due_date, subtotal, tax, total, paid_amount, notes.
- `invoice_line_items` — invoice_id, description, quantity, unit_price, amount, optional
  time_entry_id and expense_entry_id FKs, sort_order.
- `trust_ledger` — **append-only** (no UPDATE or DELETE methods on the trait, ever). Columns:
  matter_id, entry_type (deposit/withdrawal/invoice_payment/refund), amount, balance_after
  (denormalized running balance), description, invoice_id, recorded_by. Withdrawals that
  would produce a negative balance must be rejected at the application layer.

**Billing service** in `src/legal/billing.rs`:
- `draft_invoice` — collect unbilled time + expense entries, build Invoice + line items
  in memory.
- `save_draft` — persist invoice + line items atomically.
- `finalize_invoice` — set status→sent, issued_date→today, stamp `billed_invoice_id` on all
  referenced time/expense entries via the `mark_*_billed` methods.
- `record_payment` — record payment, optionally draw from trust ledger.

**API:** Draft, finalize, void, payment endpoints under `/api/invoices/`, trust deposit and
ledger list under `/api/matters/{id}/trust/`.

---

## Task 6 — Audit Hardening (branch: `codex/phase2f-audit`)

Base off Task 5's branch.

**What to build:**

- `audit_events` — **append-only** DB table: event_type, actor, matter_id (nullable),
  severity (info/warn/critical), details (JSON), created_at. No updated_at.

- Add `record_with_db(event_type, actor, matter_id, severity, details, db)` async function
  alongside the existing synchronous `record()` in `src/legal/audit.rs`. It must always call
  the existing file-based `record()` first (file is authoritative), then write to DB. If the
  DB write fails, log with `tracing::warn!` and continue — never propagate the error.

- Add an `audit_db!` macro for fire-and-forget `tokio::spawn` usage at call sites.

- Instrument these events: matter created/closed, invoice finalized, payment recorded, trust
  deposit, conflict detected, conflict graph reindexed, trust withdrawal rejected.

- **API:** Replace the stub `legal_audit_list_handler` with a real implementation backed by
  `list_audit_events`. Support query filters: event_type, matter_id, severity, since, until,
  limit/offset. Return `{"events": [...], "total": N}`.
