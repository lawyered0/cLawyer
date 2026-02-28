# Task 6 — Phase 2F: Audit Hardening

**Base branch:** `codex/phase2e-billing`
**Work branch:** `codex/phase2f-audit`
**Prerequisite:** Tasks 1–5 merged (needs matter_id FK concept throughout).

## Objective

Add a DB-backed `audit_events` table as a queryable mirror of the existing file-based audit
log. Keep the hash-chain file as the tamper-evident record. Wire the existing `audit::record()`
call sites to also write to the DB. Expose a real `GET /api/legal/audit` endpoint backed by
the new table.

## New Table

### Postgres — new file: `migrations/V16__audit_events.sql`

```sql
CREATE TABLE IF NOT EXISTS audit_events (
    id          UUID PRIMARY KEY,
    event_type  TEXT NOT NULL,
    actor       TEXT NOT NULL DEFAULT 'system',
    matter_id   TEXT,             -- NULL for non-matter events
    severity    TEXT NOT NULL DEFAULT 'info'
                CHECK (severity IN ('info','warn','critical')),
    details     JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
    -- No updated_at: immutable after insert
);

CREATE INDEX IF NOT EXISTS idx_audit_events_event_type ON audit_events(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_events_matter_id ON audit_events(matter_id)
    WHERE matter_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_events_severity ON audit_events(severity)
    WHERE severity IN ('warn','critical');
```

### libSQL — append to `src/db/libsql_migrations.rs`

```sql
CREATE TABLE IF NOT EXISTS audit_events (
    id         TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    actor      TEXT NOT NULL DEFAULT 'system',
    matter_id  TEXT,
    severity   TEXT NOT NULL DEFAULT 'info',
    details    TEXT NOT NULL DEFAULT '{}',   -- JSON object
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_audit_events_event_type ON audit_events(event_type);
CREATE INDEX IF NOT EXISTS idx_audit_events_matter_id ON audit_events(matter_id);
CREATE INDEX IF NOT EXISTS idx_audit_events_created_at ON audit_events(created_at);
```

## Rust Types — add to `src/db/mod.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub event_type: String,
    pub actor: String,
    pub matter_id: Option<String>,
    pub severity: String,   // "info" | "warn" | "critical"
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
pub struct AuditFilter {
    pub event_type: Option<String>,
    pub matter_id: Option<String>,
    pub severity: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,    // default 100, max 1000
    pub offset: i64,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
// Audit — insert only (no update/delete)
async fn append_audit_event(&self, event: &AuditEvent) -> Result<(), DatabaseError>;
async fn list_audit_events(
    &self,
    filter: &AuditFilter,
) -> Result<Vec<AuditEvent>, DatabaseError>;
async fn count_audit_events(&self, filter: &AuditFilter) -> Result<i64, DatabaseError>;
```

The `// APPEND-ONLY: no update/delete methods` comment applies here too. Add it above the
`append_audit_event` declaration.

## Wire into `src/legal/audit.rs`

The existing `audit::record()` function writes to a file. Extend it to also write to the DB
when a `Database` handle is available.

### Current signature (do not break)

```rust
pub fn record(event_type: &str, details: serde_json::Value)
```

### New overload (add alongside existing function)

```rust
pub async fn record_with_db(
    event_type: &str,
    actor: &str,
    matter_id: Option<&str>,
    severity: &str,
    details: serde_json::Value,
    db: Arc<dyn Database>,
)
```

- Always calls the existing file-based `record()` first (synchronous, infallible).
- Then builds an `AuditEvent` and calls `db.append_audit_event(&event).await`.
- If the DB write fails, log the error with `tracing::warn!` but do NOT propagate the error
  (the file is authoritative; a DB write failure must not block the operation being audited).

### Macro for convenience (add to `src/legal/audit.rs`)

```rust
/// Fire-and-forget DB audit, swallowing errors after logging.
/// Usage: audit_db!(db, "event_type", actor, matter_id, "severity", details_json).
#[macro_export]
macro_rules! audit_db {
    ($db:expr, $event_type:expr, $actor:expr, $matter_id:expr, $severity:expr, $details:expr) => {
        tokio::spawn(crate::legal::audit::record_with_db(
            $event_type,
            $actor,
            $matter_id,
            $severity,
            $details,
            Arc::clone(&$db),
        ))
    };
}
```

### Call sites to instrument

Add `record_with_db` / `audit_db!` calls at these locations (read each file first; adapt to
actual function signatures):

| File | Event | Severity |
|------|-------|----------|
| `src/channels/web/server.rs` — `matters_create_handler` | `matter.created` | info |
| `src/channels/web/server.rs` — `matters_close_handler` | `matter.closed` | info |
| `src/channels/web/server.rs` — `invoices_finalize_handler` | `invoice.finalized` | info |
| `src/channels/web/server.rs` — `invoices_payment_handler` | `invoice.payment_recorded` | info |
| `src/channels/web/server.rs` — `trust_deposit_handler` | `trust.deposit` | info |
| `src/legal/matter.rs` — `detect_conflict_with_store` when hits found | `conflict.detected` | warn |
| `src/legal/matter.rs` — `reindex_conflict_graph` on completion | `conflict_graph.reindexed` | info |
| `src/db/mod.rs` — `append_trust_entry` on withdrawal rejected (negative balance) | `trust.withdrawal_rejected` | warn |

Include the `matter_id` where known. Actor should be the authenticated user ID from the
request context, or `"system"` for background operations.

## Real Audit API Endpoint

Replace the stub `legal_audit_list_handler` in `src/channels/web/server.rs`:

```
GET /api/legal/audit
  query params:
    event_type  (string)
    matter_id   (string)
    severity    (string: info|warn|critical)
    since       (ISO-8601 datetime)
    until       (ISO-8601 datetime)
    limit       (int, default 100, max 1000)
    offset      (int, default 0)

response:
  {
    "events": [ AuditEvent, ... ],
    "total": <count matching filter without limit/offset>
  }
```

Parse query params into an `AuditFilter`. Call `db.list_audit_events(&filter)` and
`db.count_audit_events(&filter)` in parallel via `tokio::join!`.

## DB Access in Audit

`record_with_db` needs an `Arc<dyn Database>`. Pass it in at every call site.
Do NOT add a global DB handle (no `static Arc<dyn Database>`). Every caller must supply it
explicitly from its own `AppState` or function argument.

The `AppState` struct in `src/channels/web/server.rs` already holds a `db: Arc<dyn Database>`.
For the billing / matter handlers added in earlier tasks, extract it via `state.db.clone()`.

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- Audit table is append-only: document with `// APPEND-ONLY` comment, no delete/update methods.
- `record_with_db` must never propagate DB errors to the caller.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test audit
cargo test legal::audit
```
