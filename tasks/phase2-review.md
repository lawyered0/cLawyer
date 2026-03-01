# Phase 2 Review — Tasks 0-6

Reviewed at commit 408b7b4 (codex/phase2f-audit).

## Status: PASS

### Gate results
| Check | Result |
|-------|--------|
| cargo fmt | ✅ clean |
| cargo clippy --all --all-features | ✅ 0 warnings |
| cargo check --no-default-features --features libsql | ✅ clean |
| cargo test --lib | ✅ 1691 passed, 0 failed |
| cargo test --test ws_gateway_integration | ✅ 14/14 |

### Scope verified
- Task 0: Conflict reindex serialization + atomic seed
- Task 1: Client/matter CRUD (both DB backends)
- Task 2: Matter deadlines + court rules calendar
- Task 3: Document management + docgen
- Task 4: Time and expense tracking
- Task 5: Billing, invoices, trust ledger (atomic balance fix)
- Task 6: DB-backed audit events (file-first, hash chain)

### Bugs found & fixed (408b7b4)
1. `record_payment` had no status guard — voided/draft invoices could
   receive payments. Fixed: guard on `InvoiceStatus::Sent`.
2. `invoices_void_handler` had no status guard — paid invoices could be
   voided. Fixed: fetch-then-check returning 409 Conflict for non-draft/non-sent.
3. Three regression tests added covering both paths.

### No other issues found
- All 14 legal store traits implemented in both Postgres and libSQL backends
- Database supertrait bounds complete
- Audit instrumentation covers all required legal events via record_with_db
- Table parity between Postgres migrations and libSQL consolidated schema
- No unwrap/expect in production paths of new code
- No super:: imports outside test modules

## Follow-up: Extended-Usage Robustness Review

Reviewed at commit `3d9276e` (review/phase2-full, tracking codex/phase2f-audit).

### Additional bugs found and fixed (3d9276e)

**Bug 3 — Audit hash-chain state-before-write race (`audit.rs:103`)**
`*state = Some(hash)` advanced the in-memory chain head before `writeln!`
succeeded. A disk-write failure (full disk, permission tightened mid-run)
would permanently corrupt the chain for the process lifetime: every
subsequent event would reference a hash that was never persisted.
Fix: compute `pending_hash`, only advance `*state` after `writeln!` returns `Ok`.

**Bug 4 — Over-payment: `paid_amount` could exceed `total`**
`apply_invoice_payment` incremented unconditionally. Two sequential partial
payments (e.g. $600 + $600 on a $1000 invoice) left `paid_amount = $1200`.
Fix (DB layer, both backends):
- Postgres: `LEAST(paid_amount + $3, total)`
- libSQL: `MIN(CAST(paid_amount...) + CAST(?3...), CAST(total...))`
Fix (application layer, `billing.rs`): remaining-balance guard returns
descriptive error "Payment of X exceeds outstanding balance of Y" before
calling the DB, so concurrent callers also get a clean failure.

### Items confirmed NOT a problem
- `parse_decimal_field` already guards `hours > 0` for time entries
- `list_invoices` doesn't exist — invoices are always fetched per-matter (bounded)
- `audit_events` list already has `limit`/`offset` in the trait signature
- Trust balance atomicity confirmed correct (advisory lock + `FOR UPDATE`)

### Final gate (1691 passed, 0 failed, 1 ignored)
