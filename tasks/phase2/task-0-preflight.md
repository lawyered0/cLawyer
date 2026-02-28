# Task 0 — Pre-flight: Fix Conflict Graph Atomicity

**Base branch:** `codex/legal-intake-conflict-graph-schema`
**Work branch:** `codex/phase2-preflight`
**Blocks:** All Phase 2 tasks. Do this first.

## Context

Two atomicity bugs in `src/legal/matter.rs` threaten conflict graph integrity. Fix both before
any Phase 2 work starts. Both backends (Postgres and libSQL) must be fixed.

## Bug 1 — `reindex_conflict_graph` has a torn-read window

`reset_conflict_graph()` is now correctly transactional (one tx resets all tables). But the
outer reindex loop calls it, then seeds each matter one at a time in separate transactions:

```
reset_conflict_graph()       ← single tx, clears everything
for matter in matters:
    seed_matter_parties()    ← separate tx per matter
    upsert_party_aliases()   ← separate tx per matter
```

A concurrent `find_conflict_hits_for_names` call between any two iterations sees a
partially-rebuilt graph and may return false-clear or false-conflict results.

### Fix

Add a serialization guard around the entire reindex so in-flight conflict checks either
complete before the reset or wait until the reindex finishes.

Use a `tokio::sync::Mutex<()>` (not `RwLock` — this is a full exclusion, not a readers/writer
split) stored in a `static LazyLock`. Acquire the lock before calling `reset_conflict_graph()`
and release it only after the last `upsert_party_aliases()` call in the loop:

```rust
static REINDEX_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
```

In `reindex_conflict_graph`:
```rust
let _guard = REINDEX_LOCK.lock().await;
// ... existing reset + seed loop ...
```

In `detect_conflict_with_store` and `detect_conflict` (the two conflict-check paths), do NOT
acquire the same lock — reads must not stall normal operations. The lock is write-side only.
The partial-read hazard is acceptable for the brief seed-per-matter window (conflict checks
are advisory; the final clearance gate is the authoritative decision point). The lock prevents
two concurrent reindex operations from interleaving, which is the dangerous case.

## Bug 2 — Per-entry seed + alias pair is not atomic

Inside the reindex loop, for each global conflict entry:

```rust
seed_matter_parties(matter_id, parties).await?;   // party row committed
upsert_party_aliases(party_id, aliases).await?;   // aliases committed separately
```

If the process crashes (or returns an error) between the two calls, a party row exists with no
aliases, causing missed matches.

### Fix

Add a new `Database` trait method that seeds one conflict entry atomically:

```rust
async fn seed_conflict_entry(
    &self,
    matter_id: &str,
    canonical_name: &str,
    party_type: &str,   // "individual" | "entity"
    role: PartyRole,
    aliases: &[String],
) -> Result<(), DatabaseError>;
```

Implement it in both backends as a single transaction: upsert the party row, upsert all
aliases, upsert the matter_parties link — all within one BEGIN/COMMIT.

Replace the two-call pattern in `reindex_conflict_graph` with one call to
`seed_conflict_entry` per entry. Remove the now-redundant calls to `seed_matter_parties` and
`upsert_party_aliases` from the reindex loop (keep them available on the trait for other
callers that legitimately need the separate operations).

## Files to touch

- `src/db/mod.rs` — add `seed_conflict_entry` trait method + REINDEX_LOCK note (in a comment)
- `src/db/postgres.rs` — implement `seed_conflict_entry`
- `src/db/libsql_backend.rs` — implement `seed_conflict_entry`
- `src/legal/matter.rs` — add `REINDEX_LOCK`, use it in `reindex_conflict_graph`, replace two-call pattern with `seed_conflict_entry`

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo test legal
cargo test conflict
```

All tests must pass. No new warnings.
