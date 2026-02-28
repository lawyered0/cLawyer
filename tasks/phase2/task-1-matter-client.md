# Task 1 — Phase 2A: Matter & Client Normalization

**Base branch:** `codex/phase2-preflight`
**Work branch:** `codex/phase2a-matter-client`
**Prerequisite:** Task 0 merged.

## Objective

Move matter metadata from workspace files into the database. Add a normalized `clients` table
so client is a first-class entity (not a free string). Add `matter_tasks` and `matter_notes`
tables. Wire up `Database` trait methods for all new tables, implement in both backends, and
expose CRUD API endpoints in the web gateway.

Existing workspace-file matters (e.g. `matters/{id}/metadata.json`) continue to work.
On first startup after migration, a reindex pass creates DB rows for them.

## New Tables

### Postgres — new file: `migrations/V11__matter_client.sql`

```sql
CREATE TABLE IF NOT EXISTS clients (
    id            UUID PRIMARY KEY,
    name          TEXT NOT NULL,
    name_normalized TEXT NOT NULL UNIQUE,
    type          TEXT NOT NULL DEFAULT 'individual'
                  CHECK (type IN ('individual', 'entity')),
    email         TEXT,
    phone         TEXT,
    address       TEXT,
    notes         TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_clients_name_normalized ON clients(name_normalized);
CREATE INDEX IF NOT EXISTS idx_clients_name_trgm ON clients USING GIN (name_normalized gin_trgm_ops);

CREATE TABLE IF NOT EXISTS matters (
    id            TEXT PRIMARY KEY,          -- matches existing matter_id convention
    client_id     UUID REFERENCES clients(id),
    status        TEXT NOT NULL DEFAULT 'active'
                  CHECK (status IN ('intake','active','pending','closed','archived')),
    stage         TEXT,
    practice_area TEXT,
    jurisdiction  TEXT,
    opened_at     TIMESTAMPTZ,
    closed_at     TIMESTAMPTZ,
    assigned_to   TEXT[] NOT NULL DEFAULT '{}',
    custom_fields JSONB NOT NULL DEFAULT '{}',
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_matters_client_id ON matters(client_id);
CREATE INDEX IF NOT EXISTS idx_matters_status ON matters(status);
CREATE INDEX IF NOT EXISTS idx_matters_practice_area ON matters(practice_area);

CREATE TABLE IF NOT EXISTS matter_tasks (
    id          UUID PRIMARY KEY,
    matter_id   TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'open'
                CHECK (status IN ('open','in_progress','done','blocked')),
    assignee    TEXT,
    due_at      TIMESTAMPTZ,
    blocked_by  UUID[] NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_matter_tasks_matter_id ON matter_tasks(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_tasks_status ON matter_tasks(status);

CREATE TABLE IF NOT EXISTS matter_notes (
    id        UUID PRIMARY KEY,
    matter_id TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    author    TEXT NOT NULL,
    body      TEXT NOT NULL,
    pinned    BOOL NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_matter_notes_matter_id ON matter_notes(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_notes_pinned ON matter_notes(matter_id, pinned);
```

### libSQL — append to `src/db/libsql_migrations.rs`

Follow the exact patterns already in that file:
- UUID → TEXT
- TIMESTAMPTZ → TEXT (store ISO-8601 via `chrono::Utc::now().to_rfc3339()`)
- JSONB → TEXT (store via `serde_json::to_string`)
- TEXT[] → TEXT (store as JSON array string)
- No `REFERENCES` (libSQL doesn't enforce FK; add a comment noting the logical FK)

```sql
CREATE TABLE IF NOT EXISTS clients (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    name_normalized  TEXT NOT NULL UNIQUE,
    type             TEXT NOT NULL DEFAULT 'individual',
    email            TEXT,
    phone            TEXT,
    address          TEXT,
    notes            TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_clients_name_normalized ON clients(name_normalized);

CREATE TABLE IF NOT EXISTS matters (
    id            TEXT PRIMARY KEY,
    client_id     TEXT,               -- logical FK → clients.id
    status        TEXT NOT NULL DEFAULT 'active',
    stage         TEXT,
    practice_area TEXT,
    jurisdiction  TEXT,
    opened_at     TEXT,
    closed_at     TEXT,
    assigned_to   TEXT NOT NULL DEFAULT '[]',   -- JSON array
    custom_fields TEXT NOT NULL DEFAULT '{}',   -- JSON object
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_matters_client_id ON matters(client_id);
CREATE INDEX IF NOT EXISTS idx_matters_status ON matters(status);

CREATE TABLE IF NOT EXISTS matter_tasks (
    id          TEXT PRIMARY KEY,
    matter_id   TEXT NOT NULL,        -- logical FK → matters.id
    title       TEXT NOT NULL,
    description TEXT,
    status      TEXT NOT NULL DEFAULT 'open',
    assignee    TEXT,
    due_at      TEXT,
    blocked_by  TEXT NOT NULL DEFAULT '[]',  -- JSON array of task IDs
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_matter_tasks_matter_id ON matter_tasks(matter_id);

CREATE TABLE IF NOT EXISTS matter_notes (
    id        TEXT PRIMARY KEY,
    matter_id TEXT NOT NULL,          -- logical FK → matters.id
    author    TEXT NOT NULL,
    body      TEXT NOT NULL,
    pinned    INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_matter_notes_matter_id ON matter_notes(matter_id);
```

## Rust Types — add to `src/db/mod.rs`

Follow the style of `ConflictClearanceRecord` and `ConflictHit` already in that file.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRecord {
    pub id: Uuid,
    pub name: String,
    pub name_normalized: String,
    #[serde(rename = "type")]
    pub client_type: String,   // "individual" | "entity"
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterRow {
    pub id: String,
    pub client_id: Option<Uuid>,
    pub status: String,
    pub stage: Option<String>,
    pub practice_area: Option<String>,
    pub jurisdiction: Option<String>,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub assigned_to: Vec<String>,
    pub custom_fields: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterTask {
    pub id: Uuid,
    pub matter_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,   // "open" | "in_progress" | "done" | "blocked"
    pub assignee: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub blocked_by: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterNote {
    pub id: Uuid,
    pub matter_id: String,
    pub author: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct MatterFilter {
    pub status: Option<String>,
    pub practice_area: Option<String>,
    pub client_id: Option<Uuid>,
    pub assignee: Option<String>,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
// --- Clients ---
async fn create_client(&self, record: &ClientRecord) -> Result<(), DatabaseError>;
async fn get_client(&self, id: Uuid) -> Result<Option<ClientRecord>, DatabaseError>;
async fn find_client_by_name(&self, normalized: &str) -> Result<Option<ClientRecord>, DatabaseError>;
async fn update_client(&self, record: &ClientRecord) -> Result<(), DatabaseError>;
async fn list_clients(&self, limit: i64, offset: i64) -> Result<Vec<ClientRecord>, DatabaseError>;
async fn search_clients(&self, query: &str) -> Result<Vec<ClientRecord>, DatabaseError>;

// --- Matters ---
async fn create_matter(&self, matter: &MatterRow) -> Result<(), DatabaseError>;
async fn get_matter(&self, id: &str) -> Result<Option<MatterRow>, DatabaseError>;
async fn update_matter(&self, matter: &MatterRow) -> Result<(), DatabaseError>;
async fn list_matters(
    &self,
    filter: &MatterFilter,
    limit: i64,
    offset: i64,
) -> Result<Vec<MatterRow>, DatabaseError>;
async fn close_matter(&self, id: &str, closed_at: DateTime<Utc>) -> Result<(), DatabaseError>;

// --- Tasks ---
async fn create_task(&self, task: &MatterTask) -> Result<(), DatabaseError>;
async fn get_task(&self, id: Uuid) -> Result<Option<MatterTask>, DatabaseError>;
async fn update_task(&self, task: &MatterTask) -> Result<(), DatabaseError>;
async fn list_tasks_for_matter(&self, matter_id: &str) -> Result<Vec<MatterTask>, DatabaseError>;
async fn delete_task(&self, id: Uuid) -> Result<bool, DatabaseError>;

// --- Notes ---
async fn create_note(&self, note: &MatterNote) -> Result<(), DatabaseError>;
async fn list_notes_for_matter(&self, matter_id: &str) -> Result<Vec<MatterNote>, DatabaseError>;
async fn delete_note(&self, id: Uuid) -> Result<bool, DatabaseError>;
```

Implement all methods in both `src/db/postgres.rs` and `src/db/libsql_backend.rs`.
Follow the existing `record_conflict_clearance` / `find_conflict_hits_for_names` implementations
in each file as a style guide.

## Matter Reindex on Startup

Add `reindex_matters_from_workspace` in `src/legal/matter.rs`:

```rust
pub async fn reindex_matters_from_workspace(
    config: &LegalConfig,
    db: Arc<dyn Database>,
) -> Result<usize, crate::error::WorkspaceError>
```

- Walk all matter directories using the existing `matter_root` config.
- For each matter: read `MatterMetadata` from the workspace file.
- Check if the matter already exists in DB via `db.get_matter(&matter_id)`.
- If not: upsert a `ClientRecord` (lookup by `normalize_party_name(client)` first, create if
  absent), then create the `MatterRow`.
- Return the count of matters inserted.
- Call this from `main.rs` startup (after DB init, before channel start), guarded by
  `config.legal.enabled`.

## API Endpoints — add to `src/channels/web/server.rs`

These replace the stub handlers that currently exist or add new routes:

| Method | Path | Handler | Notes |
|--------|------|---------|-------|
| GET | `/api/clients` | `clients_list_handler` | query: `q` (search), `limit`, `offset` |
| POST | `/api/clients` | `clients_create_handler` | body: `{name, type, email, phone, address, notes}` |
| GET | `/api/clients/{id}` | `clients_get_handler` | |
| PATCH | `/api/clients/{id}` | `clients_update_handler` | partial update |
| GET | `/api/matters` | `matters_list_handler` | query: `status`, `practice_area`, `client_id`, `assignee`, `limit`, `offset` |
| POST | `/api/matters` | `matters_create_handler` | runs conflict check gate; body matches `MatterRow` fields |
| GET | `/api/matters/{id}` | `matters_get_handler` | returns matter + client + tasks (open only) + pinned notes |
| PATCH | `/api/matters/{id}` | `matters_update_handler` | partial update: status, stage, custom_fields, assigned_to |
| POST | `/api/matters/{id}/close` | `matters_close_handler` | sets closed_at, status → closed |
| GET | `/api/matters/{id}/tasks` | `matter_tasks_list_handler` | |
| POST | `/api/matters/{id}/tasks` | `matter_tasks_create_handler` | |
| PATCH | `/api/tasks/{id}` | `matter_tasks_update_handler` | |
| DELETE | `/api/tasks/{id}` | `matter_tasks_delete_handler` | |
| GET | `/api/matters/{id}/notes` | `matter_notes_list_handler` | |
| POST | `/api/matters/{id}/notes` | `matter_notes_create_handler` | |
| DELETE | `/api/notes/{id}` | `matter_notes_delete_handler` | |

All handlers follow the pattern of the existing `matter_dashboard_handler`:
- Extract `State(state): State<Arc<AppState>>` and `auth: AuthBearer`.
- Return `Json(...)` on success, `StatusCode::NOT_FOUND` or `StatusCode::BAD_REQUEST` on error.
- Log errors with `tracing::warn!` or `tracing::error!` before returning.
- Do not use `.unwrap()`.

`matters_create_handler` must call `detect_conflict_with_store` and return HTTP 409 with a
conflict summary JSON body if any conflict hits exist and have not been cleared.

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- All new DB methods must appear in the trait first, then both backend implementations.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test
```
