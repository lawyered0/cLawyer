# Task 3 — Phase 2C: Document Management

**Base branch:** `codex/phase2b-deadlines`
**Work branch:** `codex/phase2c-documents`
**Prerequisite:** Task 1 merged. `matters` table exists.

## Objective

Link documents to matters with versioning and categories. Add a template system that renders
Tera templates against matter + client data and saves the result as a workspace document.
Extend the existing `matter_documents_handler` stub with real persistence.

Do NOT replace the workspace `memory_documents` system. All document content lives there.
This task adds a linking table and version history on top.

## New Tables

### Postgres — new file: `migrations/V13__matter_documents.sql`

```sql
CREATE TABLE IF NOT EXISTS matter_documents (
    id              UUID PRIMARY KEY,
    matter_id       TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    document_id     UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    display_name    TEXT NOT NULL,
    category        TEXT NOT NULL DEFAULT 'internal'
                    CHECK (category IN
                      ('pleading','correspondence','contract',
                       'filing','evidence','internal')),
    uploaded_by     TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (matter_id, document_id)
);

CREATE INDEX IF NOT EXISTS idx_matter_documents_matter_id ON matter_documents(matter_id);

CREATE TABLE IF NOT EXISTS document_versions (
    id                  UUID PRIMARY KEY,
    matter_document_id  UUID NOT NULL REFERENCES matter_documents(id) ON DELETE CASCADE,
    document_id         UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    version_number      INT NOT NULL,
    label               TEXT,    -- "draft" | "filed" | "executed" | free text
    created_by          TEXT NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (matter_document_id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_document_versions_matter_document_id
    ON document_versions(matter_document_id);

CREATE TABLE IF NOT EXISTS document_templates (
    id           UUID PRIMARY KEY,
    name         TEXT NOT NULL UNIQUE,
    practice_area TEXT,
    body         TEXT NOT NULL,     -- Tera template source
    variables    JSONB NOT NULL DEFAULT '[]',
                 -- [{name: string, label: string, required: bool}]
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### libSQL — append to `src/db/libsql_migrations.rs`

```sql
CREATE TABLE IF NOT EXISTS matter_documents (
    id           TEXT PRIMARY KEY,
    matter_id    TEXT NOT NULL,     -- logical FK → matters.id
    document_id  TEXT NOT NULL,     -- logical FK → memory_documents.id
    display_name TEXT NOT NULL,
    category     TEXT NOT NULL DEFAULT 'internal',
    uploaded_by  TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (matter_id, document_id)
);
CREATE INDEX IF NOT EXISTS idx_matter_documents_matter_id ON matter_documents(matter_id);

CREATE TABLE IF NOT EXISTS document_versions (
    id                 TEXT PRIMARY KEY,
    matter_document_id TEXT NOT NULL,    -- logical FK → matter_documents.id
    document_id        TEXT NOT NULL,    -- logical FK → memory_documents.id
    version_number     INTEGER NOT NULL,
    label              TEXT,
    created_by         TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    UNIQUE (matter_document_id, version_number)
);
CREATE INDEX IF NOT EXISTS idx_document_versions_matter_document_id
    ON document_versions(matter_document_id);

CREATE TABLE IF NOT EXISTS document_templates (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    practice_area TEXT,
    body          TEXT NOT NULL,
    variables     TEXT NOT NULL DEFAULT '[]',   -- JSON array
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
```

## Rust Types — add to `src/db/mod.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterDocument {
    pub id: Uuid,
    pub matter_id: String,
    pub document_id: Uuid,
    pub display_name: String,
    pub category: String,
    pub uploaded_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersion {
    pub id: Uuid,
    pub matter_document_id: Uuid,
    pub document_id: Uuid,
    pub version_number: i32,
    pub label: Option<String>,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentTemplate {
    pub id: Uuid,
    pub name: String,
    pub practice_area: Option<String>,
    pub body: String,
    pub variables: serde_json::Value,   // array of variable descriptors
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
// Matter ↔ document links
async fn attach_document_to_matter(
    &self,
    link: &MatterDocument,
) -> Result<(), DatabaseError>;
async fn list_documents_for_matter(
    &self,
    matter_id: &str,
) -> Result<Vec<MatterDocument>, DatabaseError>;
async fn detach_document_from_matter(&self, id: Uuid) -> Result<bool, DatabaseError>;

// Document versions
async fn create_document_version(
    &self,
    version: &DocumentVersion,
) -> Result<(), DatabaseError>;
async fn list_document_versions(
    &self,
    matter_document_id: Uuid,
) -> Result<Vec<DocumentVersion>, DatabaseError>;

// Templates
async fn create_template(&self, t: &DocumentTemplate) -> Result<(), DatabaseError>;
async fn get_template(&self, id: Uuid) -> Result<Option<DocumentTemplate>, DatabaseError>;
async fn get_template_by_name(&self, name: &str) -> Result<Option<DocumentTemplate>, DatabaseError>;
async fn list_templates(
    &self,
    practice_area: Option<&str>,
) -> Result<Vec<DocumentTemplate>, DatabaseError>;
async fn update_template(&self, t: &DocumentTemplate) -> Result<(), DatabaseError>;
async fn delete_template(&self, id: Uuid) -> Result<bool, DatabaseError>;
```

## Document Generation Engine — new file: `src/legal/docgen.rs`

Uses the `tera` crate for template rendering. Add `tera = "1"` to `Cargo.toml` if not already
present. Do not add any other new dependencies.

```rust
use tera::{Context, Tera};

/// Context passed to the template engine for rendering.
/// All fields are optional; missing fields produce empty strings in the output.
#[derive(Debug, Serialize)]
pub struct DocgenContext {
    pub matter_id: String,
    pub matter_status: Option<String>,
    pub practice_area: Option<String>,
    pub jurisdiction: Option<String>,
    pub client_name: Option<String>,
    pub client_type: Option<String>,
    pub client_email: Option<String>,
    pub opened_at: Option<String>,      // formatted date
    pub today: String,                  // "YYYY-MM-DD"
    pub extra: serde_json::Value,       // caller-supplied overrides (object)
}

/// Render a template body against the provided context.
/// Returns the rendered string.
pub fn render_template(
    body: &str,
    ctx: &DocgenContext,
) -> Result<String, crate::error::WorkspaceError>

/// Build a DocgenContext from a MatterRow + optional ClientRecord.
pub fn build_context(
    matter: &MatterRow,
    client: Option<&ClientRecord>,
    extra: serde_json::Value,
) -> DocgenContext
```

### Error mapping

Map `tera::Error` to `WorkspaceError::InvalidContent { reason }`.

## API Endpoints — add to `src/channels/web/server.rs`

Replace the existing `matter_documents_handler` stub and the `matter_templates_handler` stub.

| Method | Path | Handler | Notes |
|--------|------|---------|-------|
| GET | `/api/matters/{id}/documents` | `matter_documents_list_handler` | returns `Vec<MatterDocument>` |
| POST | `/api/matters/{id}/documents` | `matter_documents_attach_handler` | body: `{document_path, display_name, category, uploaded_by}`; creates workspace doc if path is new, then links |
| DELETE | `/api/matter-documents/{id}` | `matter_documents_detach_handler` | removes link only; does not delete workspace doc |
| GET | `/api/matter-documents/{id}/versions` | `document_versions_list_handler` | |
| POST | `/api/matter-documents/{id}/versions` | `document_versions_create_handler` | body: `{document_path, label, created_by}`; bumps version_number |
| GET | `/api/templates` | `templates_list_handler` | query: `practice_area` |
| POST | `/api/templates` | `templates_create_handler` | |
| GET | `/api/templates/{id}` | `templates_get_handler` | |
| PATCH | `/api/templates/{id}` | `templates_update_handler` | |
| DELETE | `/api/templates/{id}` | `templates_delete_handler` | |
| POST | `/api/documents/generate` | `document_generate_handler` | body: `{template_id, matter_id, extra: {}}` → renders template, writes result to workspace at `matters/{matter_id}/generated/{template_name}_{timestamp}.md`, attaches as MatterDocument, returns `{document_path, content}` |

`matter_documents_attach_handler`: if `document_path` refers to a workspace path that already
has a `memory_document` row, look it up by path via `db.get_document_by_path`. If not found,
create a new empty one via `db.get_or_create_document_by_path` and write the provided content.
Then insert a `MatterDocument` link.

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- `tera` template errors must never panic; always return `Result`.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test legal::docgen
cargo test document
```
