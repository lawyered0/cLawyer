# Task 2 — Phase 2B: Calendar & Deadlines

**Base branch:** `codex/phase2a-matter-client`
**Work branch:** `codex/phase2b-deadlines`
**Prerequisite:** Task 1 merged. `matters` table exists.

## Objective

Persist matter deadlines in the database. Add a court-rule calculator that derives computed
deadlines from trigger dates and named rules. Wire reminder generation into the existing
`routines` system so deadlines fire routine jobs at the right time.

## New Table

### Postgres — new file: `migrations/V12__matter_deadlines.sql`

```sql
CREATE TABLE IF NOT EXISTS matter_deadlines (
    id            UUID PRIMARY KEY,
    matter_id     TEXT NOT NULL REFERENCES matters(id) ON DELETE CASCADE,
    title         TEXT NOT NULL,
    deadline_type TEXT NOT NULL
                  CHECK (deadline_type IN
                    ('court_date','filing','statute_of_limitations',
                     'response_due','discovery_cutoff','internal')),
    due_at        TIMESTAMPTZ NOT NULL,
    completed_at  TIMESTAMPTZ,
    reminder_days INT[] NOT NULL DEFAULT '{}',
    rule_ref      TEXT,           -- e.g. "FRCP 26(a)(1)" if court-rule derived
    computed_from UUID REFERENCES matter_deadlines(id) ON DELETE SET NULL,
    task_id       UUID REFERENCES matter_tasks(id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_matter_deadlines_matter_id ON matter_deadlines(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_deadlines_due_at ON matter_deadlines(due_at)
    WHERE completed_at IS NULL;
```

### libSQL — append to `src/db/libsql_migrations.rs`

```sql
CREATE TABLE IF NOT EXISTS matter_deadlines (
    id            TEXT PRIMARY KEY,
    matter_id     TEXT NOT NULL,           -- logical FK → matters.id
    title         TEXT NOT NULL,
    deadline_type TEXT NOT NULL,
    due_at        TEXT NOT NULL,           -- ISO-8601
    completed_at  TEXT,
    reminder_days TEXT NOT NULL DEFAULT '[]',  -- JSON int array
    rule_ref      TEXT,
    computed_from TEXT,                    -- logical FK → matter_deadlines.id
    task_id       TEXT,                    -- logical FK → matter_tasks.id
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_matter_deadlines_matter_id ON matter_deadlines(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_deadlines_due_at ON matter_deadlines(due_at);
```

## Rust Type — add to `src/db/mod.rs`

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterDeadline {
    pub id: Uuid,
    pub matter_id: String,
    pub title: String,
    pub deadline_type: String,
    pub due_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub reminder_days: Vec<i32>,
    pub rule_ref: Option<String>,
    pub computed_from: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

## Database Trait Methods — add to `src/db/mod.rs`

```rust
async fn create_deadline(&self, deadline: &MatterDeadline) -> Result<(), DatabaseError>;
async fn get_deadline(&self, id: Uuid) -> Result<Option<MatterDeadline>, DatabaseError>;
async fn update_deadline(&self, deadline: &MatterDeadline) -> Result<(), DatabaseError>;
async fn list_deadlines_for_matter(
    &self,
    matter_id: &str,
    include_completed: bool,
) -> Result<Vec<MatterDeadline>, DatabaseError>;
async fn complete_deadline(
    &self,
    id: Uuid,
    completed_at: DateTime<Utc>,
) -> Result<(), DatabaseError>;
async fn list_upcoming_deadlines(
    &self,
    before: DateTime<Utc>,
) -> Result<Vec<MatterDeadline>, DatabaseError>;
async fn delete_deadline(&self, id: Uuid) -> Result<bool, DatabaseError>;
```

Implement in both backends following the `create_task` / `list_tasks_for_matter` style
from Task 1.

## Court-Rule Calculator — new file: `src/legal/calendar.rs`

Provides a pluggable rule engine that derives a `MatterDeadline` from a trigger date and a
named rule. Rules are loaded from a TOML file at startup; the engine falls back to the bundled
default rules if no custom file is found.

### Rule format (bundled as `src/legal/court_rules.toml`)

```toml
[[rules]]
id          = "FRCP_26a1"
ref         = "FRCP 26(a)(1)"
description = "Initial disclosures — 14 days after Rule 26(f) conference"
offset_days = 14
offset_type = "calendar"   # "calendar" | "court" (business days, skipping weekends)
deadline_type = "filing"

[[rules]]
id          = "FRCP_12a1"
ref         = "FRCP 12(a)(1)"
description = "Answer to complaint — 21 days after service"
offset_days = 21
offset_type = "calendar"
deadline_type = "response_due"

[[rules]]
id          = "FRCP_56c1"
ref         = "FRCP 56(c)(1)"
description = "Motion for summary judgment — filed at least 30 days before hearing"
offset_days = -30
offset_type = "calendar"
deadline_type = "filing"

[[rules]]
id          = "CA_CCP_412_20"
ref         = "CCP § 412.20"
description = "CA service of summons — 30 days to respond"
offset_days = 30
offset_type = "calendar"
deadline_type = "response_due"
```

Start with at least these four bundled rules. Add more as needed.

### Public API

```rust
/// A loaded rule ready for calculation.
#[derive(Debug, Clone, Deserialize)]
pub struct CourtRule {
    pub id: String,
    pub reference: String,       // field name "ref" clashes with Rust keyword; use alias
    pub description: String,
    pub offset_days: i32,        // negative = before trigger
    pub offset_type: String,     // "calendar" | "court"
    pub deadline_type: String,
}

/// Load all rules: bundled defaults merged with optional custom file.
pub fn load_rules(custom_path: Option<&std::path::Path>) -> Vec<CourtRule>;

/// Find a rule by ID.
pub fn find_rule<'a>(rules: &'a [CourtRule], id: &str) -> Option<&'a CourtRule>;

/// Compute the deadline date from a trigger date and a rule.
/// "court" offset skips Saturdays and Sundays (no holiday calendar in phase 2).
pub fn apply_rule(rule: &CourtRule, trigger: DateTime<Utc>) -> DateTime<Utc>;

/// Build a MatterDeadline from a rule + trigger, ready to insert.
pub fn deadline_from_rule(
    matter_id: &str,
    rule: &CourtRule,
    trigger: DateTime<Utc>,
    computed_from: Option<Uuid>,
) -> MatterDeadline;
```

## Reminder Integration

When `create_deadline` is called and `reminder_days` is non-empty, create corresponding
`Routine` rows (type `cron`) via the existing `db.create_routine()` method. Each reminder
day `d` in `reminder_days` becomes one routine that fires at `due_at - d days` (midnight UTC):

```rust
// in src/legal/calendar.rs
pub async fn register_deadline_reminders(
    db: Arc<dyn Database>,
    deadline: &MatterDeadline,
    notify_channel: &str,
    notify_user: &str,
) -> Result<(), crate::error::WorkspaceError>
```

- For each `d` in `deadline.reminder_days`:
  - Compute `fire_at = deadline.due_at - Duration::days(d as i64)`.
  - If `fire_at` is in the past, skip.
  - Build a cron expression for that exact UTC datetime (one-shot: `min hour day month *`).
  - Create a `Routine` with `name = format!("deadline-reminder-{}-{}d", deadline.id, d)`,
    `trigger = Trigger::Cron(cron_expr)`,
    `action = Action::Notify { message: format!("{}: {} due in {} days", deadline.matter_id, deadline.title, d), channel: notify_channel, user: notify_user }`.
  - Store via `db.create_routine(&routine).await?`.
- Call this from the `matters_create_deadline_handler` (see below) after inserting the deadline.

## API Endpoints — add to `src/channels/web/server.rs`

These replace the existing stub `matter_deadlines_handler`.

| Method | Path | Handler | Notes |
|--------|------|---------|-------|
| GET | `/api/matters/{id}/deadlines` | `matter_deadlines_list_handler` | query: `include_completed=true/false` |
| POST | `/api/matters/{id}/deadlines` | `matter_deadlines_create_handler` | body: MatterDeadline fields; also calls `register_deadline_reminders` |
| POST | `/api/matters/{id}/deadlines/compute` | `matter_deadlines_compute_handler` | body: `{rule_id, trigger_date}`; returns computed deadline (not persisted) |
| PATCH | `/api/deadlines/{id}` | `matter_deadlines_update_handler` | |
| POST | `/api/deadlines/{id}/complete` | `matter_deadlines_complete_handler` | |
| DELETE | `/api/deadlines/{id}` | `matter_deadlines_delete_handler` | |
| GET | `/api/legal/court-rules` | `court_rules_list_handler` | returns full bundled + custom rule list |

## Rules

- No `.unwrap()` or `.expect()` in production code.
- Use `crate::` imports, not `super::`.
- Zero `cargo clippy` warnings.
- The bundled TOML rules file must be included via `include_str!` so it's compiled into the
  binary and available without filesystem access.

## Verify

```bash
cargo fmt
cargo clippy --all --benches --tests --examples --all-features
cargo check --no-default-features --features libsql
cargo test legal::calendar
cargo test deadline
```
