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

## Simplify Pass — thread_ops.rs + matter.rs

Reviewed at commit `db3f1b35d` (codex/matter-scoped-conversations-v1).
Focus: isolation enforcement accumulation in `thread_ops.rs`; loader/sanitize/truncate
behavior in `matter.rs`.

### Findings fixed

**P1 — Lock contention in web chat handlers (`server.rs:1043`, `server.rs:1181`)**
`chat_threads_handler` and `chat_new_thread_handler` held the session lock across async
DB calls, increasing contention and latency under concurrent chat traffic.
Fix: narrowed lock scope; DB work moved outside the critical section.

**P2 — Matter-ID parsing/sanitization duplication (`policy.rs:104`, `policy.rs:115`)**
Repeated ad-hoc sanitization logic across modules increased drift risk.
Fix: consolidated into shared helpers in `policy.rs`, reused in all agent and web paths.

**P2 — Redundant control flow in matter-scope enforcement (`thread_ops.rs:191`)**
Duplicate mismatch/bound audit blocks and duplicated error formatting caused
branch proliferation.
Fix: refactored to shared helpers for clearer flow and less branch duplication.

**P3 — Redundant DB message persistence logic (`thread_ops.rs:644`)**
User/assistant persistence duplicated an ensure-then-insert flow in two call sites.
Fix: unified via a shared helper.

**P3 — Repeated workspace "write-if-missing" pattern + sequential curated-file reads
(`matter.rs:344`, `matter.rs:386`)**
Curated file loads were sequential; the write-if-missing check was copy-pasted.
Fix: introduced a reusable writer; parallelized 4-file curated-load path via
`tokio::join!`.

### No new blocking bugs found after this pass in the changed paths.

### Gate (simplify pass)
| Check | Result |
|-------|--------|
| cargo fmt --all -- --check | ✅ |
| cargo clippy --all-features --all-targets | ✅ 0 warnings |
| cargo test -- --test-threads=1 | ✅ 1713 passed, 0 failed, 1 ignored |

## Follow-on Backlog — Web Gateway Decomposition (post-`codex/web-server-feature-handler-split-v1`)

Tracked from review after server decomposition landed (`server.rs` 12,534 → ~1.2k).

### Priority order
1. `common.rs` decomposition (highest)
2. remove `#[cfg(test)]` server shims
3. migrate `server_tests.rs` tests into feature modules

### Item 1 — Split `handlers/common.rs` into focused helper modules

Current issue: `src/channels/web/handlers/common.rs` is still a large mixed helper file
with unrelated concerns.

Target modules under `src/channels/web/handlers/helpers/`:
- `legal_config.rs` (`legal_config_for_gateway*`, `matter_root_for_gateway`)
- `paths.rs` (normalize/resolve memory-write paths, traversal/protected checks)
- `audit.rs` (`record_legal_audit_event`, audit parse/mapping helpers)
- `matter_validation.rs` (matter field/ID validators)
- `parsers.rs` (date/decimal/uuid/enum parsers)
- `documents.rs` (document/template backfill/list helpers)
- `deadlines.rs` (deadline file/db mapping + reminder helpers)

Success criteria:
- `common.rs` reduced to minimal re-exports or removed.
- No behavior change in route handlers.
- Full gate green.

### Item 2 — Remove `#[cfg(test)]` shim handlers from `server.rs`

Current issue: test-only wrappers in `server.rs` forward to module handlers and keep
test architecture coupled to `super::*`.

Planned change:
- Update `src/channels/web/server_tests.rs` imports to call handlers directly from
  `crate::channels::web::handlers::*`.
- Delete test-only forwarding wrappers from `server.rs`.

Success criteria:
- `server.rs` contains startup/server concerns only.
- No test coverage loss.
- Full gate green.

### Item 3 — Move monolithic `server_tests.rs` coverage into handler-local tests

Current issue: `src/channels/web/server_tests.rs` remains very large and centralizes
feature tests.

Planned change:
- Migrate tests into `#[cfg(test)] mod tests` blocks colocated with feature handlers:
  `handlers/chat.rs`, `handlers/memory.rs`, `handlers/matters/*`, `handlers/legal.rs`, etc.
- Keep only true cross-feature gateway/bootstrap tests in `server_tests.rs`.

Success criteria:
- `server_tests.rs` becomes small and cross-cutting only.
- Test names/coverage preserved (parity check before/after).
- Full gate green.
