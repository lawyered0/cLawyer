//! PostgreSQL backend for the Database trait.
//!
//! Delegates to the existing `Store` (history) and `Repository` (workspace)
//! implementations, avoiding SQL duplication.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use deadpool_postgres::{GenericClient, Pool};
use rust_decimal::Decimal;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::config::DatabaseConfig;
use crate::context::{ActionRecord, JobContext, JobState};
use crate::db::{
    ClientRecord, ClientStore, ClientType, ConflictClearanceRecord, ConflictHit, ConversationStore,
    CreateClientParams, CreateMatterDeadlineParams, CreateMatterNoteParams, CreateMatterTaskParams,
    Database, JobStore, LegalConflictStore, MatterDeadlineRecord, MatterDeadlineStore,
    MatterDeadlineType, MatterNoteRecord, MatterNoteStore, MatterRecord, MatterStatus, MatterStore,
    MatterTaskRecord, MatterTaskStatus, MatterTaskStore, PartyRole, RoutineStore, SandboxStore,
    SettingsStore, ToolFailureStore, UpdateClientParams, UpdateMatterDeadlineParams,
    UpdateMatterNoteParams, UpdateMatterParams, UpdateMatterTaskParams, UpsertMatterParams,
    WorkspaceStore, conflict_terms_from_text, normalize_party_name,
};
use crate::error::{DatabaseError, WorkspaceError};
use crate::history::{
    ConversationMessage, ConversationSummary, JobEventRecord, LlmCallRecord, SandboxJobRecord,
    SandboxJobSummary, SettingRow, Store,
};
use crate::workspace::{
    MemoryChunk, MemoryDocument, Repository, SearchConfig, SearchResult, WorkspaceEntry,
};

/// PostgreSQL database backend.
///
/// Wraps the existing `Store` (for history/conversations/jobs/routines/settings)
/// and `Repository` (for workspace documents/chunks/search) to implement the
/// unified `Database` trait.
pub struct PgBackend {
    store: Store,
    repo: Repository,
}

impl PgBackend {
    /// Create a new PostgreSQL backend from configuration.
    pub async fn new(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let store = Store::new(config).await?;
        let repo = Repository::new(store.pool());
        Ok(Self { store, repo })
    }

    /// Get a clone of the connection pool.
    ///
    /// Useful for sharing with components that still need raw pool access.
    pub fn pool(&self) -> Pool {
        self.store.pool()
    }
}

fn normalize_input_terms(input_names: &[String]) -> Vec<String> {
    input_names
        .iter()
        .map(|name| normalize_party_name(name))
        .filter(|name| !name.is_empty())
        .collect()
}

fn sql_or_eq(column: &str, start_idx: usize, count: usize) -> String {
    (0..count)
        .map(|i| format!("{column} = ${}", start_idx + i))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn sql_values_terms(start_idx: usize, count: usize) -> String {
    (0..count)
        .map(|i| format!("(${})", start_idx + i))
        .collect::<Vec<_>>()
        .join(", ")
}

fn match_priority(matched_via: &str) -> u8 {
    if matched_via == "direct" {
        3
    } else if matched_via.starts_with("alias:") {
        2
    } else {
        1
    }
}

fn parse_opened_at_ts(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, DatabaseError> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| DatabaseError::Serialization("invalid opened_at date".to_string()))?;
        return Ok(Some(dt.and_utc()));
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(dt.with_timezone(&Utc)));
    }

    Err(DatabaseError::Serialization(format!(
        "invalid opened_at timestamp '{}'",
        raw
    )))
}

async fn upsert_party_pg<C>(conn: &C, name: &str) -> Result<Option<Uuid>, DatabaseError>
where
    C: GenericClient + Sync,
{
    let display_name = name.trim();
    if display_name.is_empty() {
        return Ok(None);
    }
    let normalized = normalize_party_name(display_name);
    if normalized.is_empty() {
        return Ok(None);
    }
    let row = conn
        .query_one(
            "INSERT INTO parties (id, name, name_normalized, party_type) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (name_normalized) DO UPDATE \
             SET name = EXCLUDED.name, updated_at = NOW() \
             RETURNING id",
            &[&Uuid::new_v4(), &display_name, &normalized, &"entity"],
        )
        .await?;
    Ok(Some(row.get::<_, Uuid>(0)))
}

fn dedupe_hits(
    rows: Vec<(String, String, String, String, String, f64)>,
    limit: usize,
) -> Vec<ConflictHit> {
    let mut best: std::collections::HashMap<(String, String, String), (u8, f64, ConflictHit)> =
        std::collections::HashMap::new();

    for (party, role_raw, matter_id, matter_status, matched_via, score) in rows {
        let Some(role) = PartyRole::from_db_value(&role_raw) else {
            continue;
        };
        let key = (party.clone(), role_raw, matter_id.clone());
        let hit = ConflictHit {
            party,
            role,
            matter_id,
            matter_status,
            matched_via: matched_via.clone(),
        };
        let priority = match_priority(&matched_via);

        match best.get(&key) {
            Some((existing_priority, existing_score, _))
                if *existing_priority > priority
                    || (*existing_priority == priority && *existing_score >= score) => {}
            _ => {
                best.insert(key, (priority, score, hit));
            }
        }
    }

    let mut hits: Vec<ConflictHit> = best.into_values().map(|(_, _, hit)| hit).collect();
    hits.sort_by(|a, b| {
        a.party
            .cmp(&b.party)
            .then_with(|| a.matter_id.cmp(&b.matter_id))
            .then_with(|| a.matched_via.cmp(&b.matched_via))
    });
    if hits.len() > limit {
        hits.truncate(limit);
    }
    hits
}

fn parse_json_string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_json_uuid_array(value: &serde_json::Value) -> Vec<Uuid> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_str())
                .filter_map(|raw| Uuid::parse_str(raw).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn row_to_client_record(row: &tokio_postgres::Row) -> Result<ClientRecord, DatabaseError> {
    let client_type_raw: String = row.get("client_type");
    let client_type = ClientType::from_db_value(&client_type_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid client_type '{}'", client_type_raw))
    })?;
    Ok(ClientRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        name_normalized: row.get("name_normalized"),
        client_type,
        email: row.get("email"),
        phone: row.get("phone"),
        address: row.get("address"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_matter_record(row: &tokio_postgres::Row) -> Result<MatterRecord, DatabaseError> {
    let status_raw: String = row.get("status");
    let status = MatterStatus::from_db_value(&status_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter status '{}'", status_raw))
    })?;
    let assigned_to_value: serde_json::Value = row.get("assigned_to");
    let custom_fields: serde_json::Value = row.get("custom_fields");
    Ok(MatterRecord {
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        client_id: row.get("client_id"),
        status,
        stage: row.get("stage"),
        practice_area: row.get("practice_area"),
        jurisdiction: row.get("jurisdiction"),
        opened_at: row.get("opened_at"),
        closed_at: row.get("closed_at"),
        assigned_to: parse_json_string_array(&assigned_to_value),
        custom_fields,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_matter_task_record(row: &tokio_postgres::Row) -> Result<MatterTaskRecord, DatabaseError> {
    let status_raw: String = row.get("status");
    let status = MatterTaskStatus::from_db_value(&status_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter task status '{}'", status_raw))
    })?;
    let blocked_by_value: serde_json::Value = row.get("blocked_by");
    Ok(MatterTaskRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        title: row.get("title"),
        description: row.get("description"),
        status,
        assignee: row.get("assignee"),
        due_at: row.get("due_at"),
        blocked_by: parse_json_uuid_array(&blocked_by_value),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_matter_note_record(row: &tokio_postgres::Row) -> MatterNoteRecord {
    MatterNoteRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        author: row.get("author"),
        body: row.get("body"),
        pinned: row.get("pinned"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn parse_json_i32_array(value: &serde_json::Value) -> Vec<i32> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_i64())
                .filter_map(|raw| i32::try_from(raw).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn row_to_matter_deadline_record(
    row: &tokio_postgres::Row,
) -> Result<MatterDeadlineRecord, DatabaseError> {
    let deadline_type_raw: String = row.get("deadline_type");
    let deadline_type = MatterDeadlineType::from_db_value(&deadline_type_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid deadline_type '{}'", deadline_type_raw))
    })?;
    let reminder_days: serde_json::Value = row.get("reminder_days");
    Ok(MatterDeadlineRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        title: row.get("title"),
        deadline_type,
        due_at: row.get("due_at"),
        completed_at: row.get("completed_at"),
        reminder_days: parse_json_i32_array(&reminder_days),
        rule_ref: row.get("rule_ref"),
        computed_from: row.get("computed_from"),
        task_id: row.get("task_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

// ==================== Database (supertrait) ====================

#[async_trait]
impl Database for PgBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        self.store.run_migrations().await
    }
}

// ==================== ConversationStore ====================

#[async_trait]
impl ConversationStore for PgBackend {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation(channel, user_id, thread_id)
            .await
    }

    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.touch_conversation(id).await
    }

    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .add_conversation_message(conversation_id, role, content)
            .await
    }

    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .ensure_conversation(id, channel, user_id, thread_id)
            .await
    }

    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError> {
        self.store
            .list_conversations_with_preview(user_id, channel, limit)
            .await
    }

    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .get_or_create_assistant_conversation(user_id, channel)
            .await
    }

    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .create_conversation_with_metadata(channel, user_id, metadata)
            .await
    }

    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError> {
        self.store
            .list_conversation_messages_paginated(conversation_id, before, limit)
            .await
    }

    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_conversation_metadata_field(id, key, value)
            .await
    }

    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_conversation_metadata(id).await
    }

    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError> {
        self.store.list_conversation_messages(conversation_id).await
    }

    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .conversation_belongs_to_user(conversation_id, user_id)
            .await
    }
}

// ==================== JobStore ====================

#[async_trait]
impl JobStore for PgBackend {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError> {
        self.store.save_job(ctx).await
    }

    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError> {
        self.store.get_job(id).await
    }

    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_job_status(id, status, failure_reason)
            .await
    }

    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError> {
        self.store.mark_job_stuck(id).await
    }

    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError> {
        self.store.get_stuck_jobs().await
    }

    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError> {
        self.store.save_action(job_id, action).await
    }

    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError> {
        self.store.get_job_actions(job_id).await
    }

    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError> {
        self.store.record_llm_call(record).await
    }

    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError> {
        self.store
            .save_estimation_snapshot(
                job_id,
                category,
                tool_names,
                estimated_cost,
                estimated_time_secs,
                estimated_value,
            )
            .await
    }

    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_estimation_actuals(id, actual_cost, actual_time_secs, actual_value)
            .await
    }
}

// ==================== SandboxStore ====================

#[async_trait]
impl SandboxStore for PgBackend {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError> {
        self.store.save_sandbox_job(job).await
    }

    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError> {
        self.store.get_sandbox_job(id).await
    }

    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs().await
    }

    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_sandbox_job_status(id, status, success, message, started_at, completed_at)
            .await
    }

    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError> {
        self.store.cleanup_stale_sandbox_jobs().await
    }

    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary().await
    }

    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError> {
        self.store.list_sandbox_jobs_for_user(user_id).await
    }

    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError> {
        self.store.sandbox_job_summary_for_user(user_id).await
    }

    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError> {
        self.store
            .sandbox_job_belongs_to_user(job_id, user_id)
            .await
    }

    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError> {
        self.store.update_sandbox_job_mode(id, mode).await
    }

    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError> {
        self.store.get_sandbox_job_mode(id).await
    }

    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.save_job_event(job_id, event_type, data).await
    }

    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError> {
        self.store.list_job_events(job_id, limit).await
    }
}

// ==================== RoutineStore ====================

#[async_trait]
impl RoutineStore for PgBackend {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.create_routine(routine).await
    }

    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine(id).await
    }

    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError> {
        self.store.get_routine_by_name(user_id, name).await
    }

    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_routines(user_id).await
    }

    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_event_routines().await
    }

    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError> {
        self.store.list_due_cron_routines().await
    }

    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError> {
        self.store.update_routine(routine).await
    }

    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store
            .update_routine_runtime(
                id,
                last_run_at,
                next_fire_at,
                run_count,
                consecutive_failures,
                state,
            )
            .await
    }

    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError> {
        self.store.delete_routine(id).await
    }

    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError> {
        self.store.create_routine_run(run).await
    }

    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError> {
        self.store
            .complete_routine_run(id, status, result_summary, tokens_used)
            .await
    }

    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError> {
        self.store.list_routine_runs(routine_id, limit).await
    }

    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError> {
        self.store.count_running_routine_runs(routine_id).await
    }

    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError> {
        self.store.link_routine_run_to_job(run_id, job_id).await
    }
}

// ==================== ToolFailureStore ====================

#[async_trait]
impl ToolFailureStore for PgBackend {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError> {
        self.store
            .record_tool_failure(tool_name, error_message)
            .await
    }

    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError> {
        self.store.get_broken_tools(threshold).await
    }

    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.mark_tool_repaired(tool_name).await
    }

    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError> {
        self.store.increment_repair_attempts(tool_name).await
    }
}

// ==================== LegalConflictStore ====================

#[async_trait]
impl LegalConflictStore for PgBackend {
    async fn find_conflict_hits_for_names(
        &self,
        input_names: &[String],
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError> {
        let terms = normalize_input_terms(input_names);
        if terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let limit = limit.min(200);
        let conn = self.store.conn().await?;
        let mut rows: Vec<(String, String, String, String, String, f64)> = Vec::new();

        let direct_clause = sql_or_eq("p.name_normalized", 1, terms.len());
        let direct_query = format!(
            "SELECT p.name, mp.role, mp.matter_id, \
                    CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                    'direct' AS matched_via, \
                    1.0::double precision AS score \
             FROM parties p \
             JOIN matter_parties mp ON mp.party_id = p.id \
             WHERE {direct_clause} \
             LIMIT ${}",
            terms.len() + 1
        );
        let direct_limit = limit as i64;
        let mut direct_params: Vec<&(dyn ToSql + Sync)> = terms
            .iter()
            .map(|term| term as &(dyn ToSql + Sync))
            .collect();
        direct_params.push(&direct_limit);
        for row in conn.query(&direct_query, &direct_params).await? {
            rows.push((
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
            ));
        }

        let alias_clause = sql_or_eq("pa.alias_normalized", 1, terms.len());
        let alias_query = format!(
            "SELECT p.name, mp.role, mp.matter_id, \
                    CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                    ('alias:' || pa.alias) AS matched_via, \
                    0.9::double precision AS score \
             FROM party_aliases pa \
             JOIN parties p ON p.id = pa.party_id \
             JOIN matter_parties mp ON mp.party_id = p.id \
             WHERE {alias_clause} \
             LIMIT ${}",
            terms.len() + 1
        );
        let alias_limit = limit as i64;
        let mut alias_params: Vec<&(dyn ToSql + Sync)> = terms
            .iter()
            .map(|term| term as &(dyn ToSql + Sync))
            .collect();
        alias_params.push(&alias_limit);
        for row in conn.query(&alias_query, &alias_params).await? {
            rows.push((
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
            ));
        }

        // Fuzzy fallback via pg_trgm similarity.
        let values = sql_values_terms(1, terms.len());
        let fuzzy_names_query = format!(
            "WITH input_terms(term) AS (VALUES {values}) \
             SELECT p.name, mp.role, mp.matter_id, \
                    CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                    ('fuzzy:' || input_terms.term) AS matched_via, \
                    similarity(p.name_normalized, input_terms.term) AS score \
             FROM input_terms \
             JOIN parties p ON p.name_normalized % input_terms.term \
             JOIN matter_parties mp ON mp.party_id = p.id \
             WHERE similarity(p.name_normalized, input_terms.term) >= 0.45 \
             LIMIT ${}",
            terms.len() + 1
        );
        let fuzzy_names_limit = limit as i64;
        let mut fuzzy_name_params: Vec<&(dyn ToSql + Sync)> = terms
            .iter()
            .map(|term| term as &(dyn ToSql + Sync))
            .collect();
        fuzzy_name_params.push(&fuzzy_names_limit);
        for row in conn.query(&fuzzy_names_query, &fuzzy_name_params).await? {
            rows.push((
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
            ));
        }

        let fuzzy_alias_query = format!(
            "WITH input_terms(term) AS (VALUES {values}) \
             SELECT p.name, mp.role, mp.matter_id, \
                    CASE WHEN mp.closed_at IS NULL THEN 'Open' ELSE 'Closed' END AS matter_status, \
                    ('fuzzy:' || input_terms.term) AS matched_via, \
                    similarity(pa.alias_normalized, input_terms.term) AS score \
             FROM input_terms \
             JOIN party_aliases pa ON pa.alias_normalized % input_terms.term \
             JOIN parties p ON p.id = pa.party_id \
             JOIN matter_parties mp ON mp.party_id = p.id \
             WHERE similarity(pa.alias_normalized, input_terms.term) >= 0.45 \
             LIMIT ${}",
            terms.len() + 1
        );
        let fuzzy_alias_limit = limit as i64;
        let mut fuzzy_alias_params: Vec<&(dyn ToSql + Sync)> = terms
            .iter()
            .map(|term| term as &(dyn ToSql + Sync))
            .collect();
        fuzzy_alias_params.push(&fuzzy_alias_limit);
        for row in conn.query(&fuzzy_alias_query, &fuzzy_alias_params).await? {
            rows.push((
                row.get(0),
                row.get(1),
                row.get(2),
                row.get(3),
                row.get(4),
                row.get(5),
            ));
        }

        Ok(dedupe_hits(rows, limit))
    }

    async fn find_conflict_hits_for_text(
        &self,
        text: &str,
        active_matter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError> {
        let terms = conflict_terms_from_text(text, active_matter);
        self.find_conflict_hits_for_names(&terms, limit).await
    }

    async fn seed_matter_parties(
        &self,
        matter_id: &str,
        client: &str,
        adversaries: &[String],
        opened_at: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let matter_id = matter_id.trim();
        if matter_id.is_empty() {
            return Err(DatabaseError::Serialization(
                "matter_id cannot be empty".to_string(),
            ));
        }

        let opened_at = parse_opened_at_ts(opened_at)?;
        let mut conn = self.store.conn().await?;

        let tx = conn.transaction().await?;

        if let Some(client_party_id) = upsert_party_pg(&tx, client).await? {
            tx.execute(
                "INSERT INTO matter_parties (id, matter_id, party_id, role, opened_at, closed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (matter_id, party_id, role) DO UPDATE \
                 SET opened_at = COALESCE(matter_parties.opened_at, EXCLUDED.opened_at), \
                     updated_at = NOW()",
                &[
                    &Uuid::new_v4(),
                    &matter_id,
                    &client_party_id,
                    &PartyRole::Client.as_str(),
                    &opened_at,
                    &Option::<DateTime<Utc>>::None,
                ],
            )
            .await?;
        }

        for name in adversaries {
            let Some(adverse_party_id) = upsert_party_pg(&tx, name).await? else {
                continue;
            };
            tx.execute(
                "INSERT INTO matter_parties (id, matter_id, party_id, role, opened_at, closed_at) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 ON CONFLICT (matter_id, party_id, role) DO UPDATE \
                 SET opened_at = COALESCE(matter_parties.opened_at, EXCLUDED.opened_at), \
                     updated_at = NOW()",
                &[
                    &Uuid::new_v4(),
                    &matter_id,
                    &adverse_party_id,
                    &PartyRole::Adverse.as_str(),
                    &opened_at,
                    &Option::<DateTime<Utc>>::None,
                ],
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn seed_conflict_entry(
        &self,
        matter_id: &str,
        canonical_name: &str,
        aliases: &[String],
        opened_at: Option<&str>,
    ) -> Result<(), DatabaseError> {
        let matter_id = matter_id.trim();
        if matter_id.is_empty() {
            return Err(DatabaseError::Serialization(
                "matter_id cannot be empty".to_string(),
            ));
        }

        let opened_at = parse_opened_at_ts(opened_at)?;
        let mut conn = self.store.conn().await?;
        let tx = conn.transaction().await?;

        let Some(party_id) = upsert_party_pg(&tx, canonical_name).await? else {
            tx.commit().await?;
            return Ok(());
        };

        tx.execute(
            "INSERT INTO matter_parties (id, matter_id, party_id, role, opened_at, closed_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (matter_id, party_id, role) DO UPDATE \
             SET opened_at = COALESCE(matter_parties.opened_at, EXCLUDED.opened_at), \
                 updated_at = NOW()",
            &[
                &Uuid::new_v4(),
                &matter_id,
                &party_id,
                &PartyRole::Adverse.as_str(),
                &opened_at,
                &Option::<DateTime<Utc>>::None,
            ],
        )
        .await?;

        let mut seen = HashSet::new();
        for alias in aliases {
            let display_alias = alias.trim();
            if display_alias.is_empty() {
                continue;
            }
            let normalized_alias = normalize_party_name(display_alias);
            if normalized_alias.is_empty() || !seen.insert(normalized_alias.clone()) {
                continue;
            }
            tx.execute(
                "INSERT INTO party_aliases (id, party_id, alias, alias_normalized) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (party_id, alias_normalized) DO UPDATE \
                 SET alias = EXCLUDED.alias, updated_at = NOW()",
                &[
                    &Uuid::new_v4(),
                    &party_id,
                    &display_alias,
                    &normalized_alias,
                ],
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn reset_conflict_graph(&self) -> Result<(), DatabaseError> {
        let mut conn = self.store.conn().await?;
        let tx = conn.transaction().await?;
        tx.execute("DELETE FROM matter_parties", &[]).await?;
        tx.execute("DELETE FROM party_aliases", &[]).await?;
        tx.execute("DELETE FROM party_relationships", &[]).await?;
        tx.execute("DELETE FROM parties", &[]).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn upsert_party_aliases(
        &self,
        canonical_name: &str,
        aliases: &[String],
    ) -> Result<(), DatabaseError> {
        if aliases.is_empty() {
            return Ok(());
        }

        let conn = self.store.conn().await?;
        let Some(party_id) = upsert_party_pg(&conn, canonical_name).await? else {
            return Ok(());
        };

        let mut seen = HashSet::new();
        for alias in aliases {
            let display_alias = alias.trim();
            if display_alias.is_empty() {
                continue;
            }
            let normalized_alias = normalize_party_name(display_alias);
            if normalized_alias.is_empty() || !seen.insert(normalized_alias.clone()) {
                continue;
            }
            conn.execute(
                "INSERT INTO party_aliases (id, party_id, alias, alias_normalized) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (party_id, alias_normalized) DO UPDATE \
                 SET alias = EXCLUDED.alias, updated_at = NOW()",
                &[
                    &Uuid::new_v4(),
                    &party_id,
                    &display_alias,
                    &normalized_alias,
                ],
            )
            .await?;
        }

        Ok(())
    }

    async fn record_conflict_clearance(
        &self,
        row: &ConflictClearanceRecord,
    ) -> Result<(), DatabaseError> {
        let conn = self.store.conn().await?;
        conn.execute(
            "INSERT INTO conflict_clearances \
             (id, matter_id, checked_by, cleared_by, decision, note, hits_json, hit_count) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            &[
                &Uuid::new_v4(),
                &row.matter_id,
                &row.checked_by,
                &row.cleared_by,
                &row.decision.as_str(),
                &row.note,
                &row.hits_json,
                &row.hit_count,
            ],
        )
        .await?;
        Ok(())
    }
}

// ==================== ClientStore ====================

#[async_trait]
impl ClientStore for PgBackend {
    async fn create_client(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO clients \
                 (id, user_id, name, name_normalized, client_type, email, phone, address, notes) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 RETURNING id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &input.name.trim(),
                    &normalized_name,
                    &input.client_type.as_str(),
                    &input.email,
                    &input.phone,
                    &input.address,
                    &input.notes,
                ],
            )
            .await?;
        row_to_client_record(&row)
    }

    async fn upsert_client_by_normalized_name(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO clients \
                 (id, user_id, name, name_normalized, client_type, email, phone, address, notes) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 ON CONFLICT (user_id, name_normalized) DO UPDATE SET \
                    name = EXCLUDED.name, \
                    client_type = EXCLUDED.client_type, \
                    email = COALESCE(EXCLUDED.email, clients.email), \
                    phone = COALESCE(EXCLUDED.phone, clients.phone), \
                    address = COALESCE(EXCLUDED.address, clients.address), \
                    notes = COALESCE(EXCLUDED.notes, clients.notes), \
                    updated_at = NOW() \
                 RETURNING id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &input.name.trim(),
                    &normalized_name,
                    &input.client_type.as_str(),
                    &input.email,
                    &input.phone,
                    &input.address,
                    &input.notes,
                ],
            )
            .await?;
        row_to_client_record(&row)
    }

    async fn list_clients(
        &self,
        user_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClientRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = if let Some(search_raw) = query {
            let search = normalize_party_name(search_raw);
            if search.is_empty() {
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients \
                     WHERE user_id = $1 \
                     ORDER BY name ASC",
                    &[&user_id],
                )
                .await?
            } else {
                let like = format!("%{search}%");
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients \
                     WHERE user_id = $1 AND (name_normalized LIKE $2 OR name_normalized % $3) \
                     ORDER BY similarity(name_normalized, $3) DESC, name ASC",
                    &[&user_id, &like, &search],
                )
                .await?
            }
        } else {
            conn.query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients \
                 WHERE user_id = $1 \
                 ORDER BY name ASC",
                &[&user_id],
            )
            .await?
        };

        rows.into_iter()
            .map(|row| row_to_client_record(&row))
            .collect()
    }

    async fn get_client(
        &self,
        user_id: &str,
        client_id: Uuid,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients \
                 WHERE user_id = $1 AND id = $2",
                &[&user_id, &client_id],
            )
            .await?;
        row.map(|row| row_to_client_record(&row)).transpose()
    }

    async fn update_client(
        &self,
        user_id: &str,
        client_id: Uuid,
        input: &UpdateClientParams,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let Some(existing) = self.get_client(user_id, client_id).await? else {
            return Ok(None);
        };

        let merged_name = input
            .name
            .as_deref()
            .unwrap_or(existing.name.as_str())
            .trim();
        let normalized_name = normalize_party_name(merged_name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }
        let merged_client_type = input.client_type.unwrap_or(existing.client_type);
        let merged_email = input.email.clone().unwrap_or(existing.email);
        let merged_phone = input.phone.clone().unwrap_or(existing.phone);
        let merged_address = input.address.clone().unwrap_or(existing.address);
        let merged_notes = input.notes.clone().unwrap_or(existing.notes);

        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "UPDATE clients SET \
                    name = $3, \
                    name_normalized = $4, \
                    client_type = $5, \
                    email = $6, \
                    phone = $7, \
                    address = $8, \
                    notes = $9, \
                    updated_at = NOW() \
                 WHERE user_id = $1 AND id = $2 \
                 RETURNING id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at",
                &[
                    &user_id,
                    &client_id,
                    &merged_name,
                    &normalized_name,
                    &merged_client_type.as_str(),
                    &merged_email,
                    &merged_phone,
                    &merged_address,
                    &merged_notes,
                ],
            )
            .await?;

        Ok(Some(row_to_client_record(&row)?))
    }

    async fn delete_client(&self, user_id: &str, client_id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.store.conn().await?;
        let deleted = conn
            .execute(
                "DELETE FROM clients WHERE user_id = $1 AND id = $2",
                &[&user_id, &client_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

// ==================== MatterStore ====================

#[async_trait]
impl MatterStore for PgBackend {
    async fn upsert_matter(
        &self,
        user_id: &str,
        input: &UpsertMatterParams,
    ) -> Result<MatterRecord, DatabaseError> {
        let assigned_to = serde_json::to_value(&input.assigned_to)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let custom_fields = if input.custom_fields.is_object() {
            input.custom_fields.clone()
        } else {
            serde_json::json!({})
        };

        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO matters \
                 (user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 ON CONFLICT (user_id, matter_id) DO UPDATE SET \
                    client_id = EXCLUDED.client_id, \
                    status = EXCLUDED.status, \
                    stage = EXCLUDED.stage, \
                    practice_area = EXCLUDED.practice_area, \
                    jurisdiction = EXCLUDED.jurisdiction, \
                    opened_at = EXCLUDED.opened_at, \
                    closed_at = EXCLUDED.closed_at, \
                    assigned_to = EXCLUDED.assigned_to, \
                    custom_fields = EXCLUDED.custom_fields, \
                    updated_at = NOW() \
                 RETURNING user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at",
                &[
                    &user_id,
                    &input.matter_id,
                    &input.client_id,
                    &input.status.as_str(),
                    &input.stage,
                    &input.practice_area,
                    &input.jurisdiction,
                    &input.opened_at,
                    &input.closed_at,
                    &assigned_to,
                    &custom_fields,
                ],
            )
            .await?;
        row_to_matter_record(&row)
    }

    async fn list_matters_db(&self, user_id: &str) -> Result<Vec<MatterRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters \
                 WHERE user_id = $1 \
                 ORDER BY matter_id ASC",
                &[&user_id],
            )
            .await?;
        rows.into_iter()
            .map(|row| row_to_matter_record(&row))
            .collect()
    }

    async fn get_matter_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters \
                 WHERE user_id = $1 AND matter_id = $2",
                &[&user_id, &matter_id],
            )
            .await?;
        row.map(|row| row_to_matter_record(&row)).transpose()
    }

    async fn update_matter(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpdateMatterParams,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let Some(existing) = self.get_matter_db(user_id, matter_id).await? else {
            return Ok(None);
        };

        let merged = UpsertMatterParams {
            matter_id: existing.matter_id.clone(),
            client_id: input.client_id.unwrap_or(existing.client_id),
            status: input.status.unwrap_or(existing.status),
            stage: input.stage.clone().unwrap_or(existing.stage),
            practice_area: input
                .practice_area
                .clone()
                .unwrap_or(existing.practice_area),
            jurisdiction: input.jurisdiction.clone().unwrap_or(existing.jurisdiction),
            opened_at: input.opened_at.unwrap_or(existing.opened_at),
            closed_at: input.closed_at.unwrap_or(existing.closed_at),
            assigned_to: input.assigned_to.clone().unwrap_or(existing.assigned_to),
            custom_fields: input
                .custom_fields
                .clone()
                .unwrap_or(existing.custom_fields),
        };

        let updated = self.upsert_matter(user_id, &merged).await?;
        Ok(Some(updated))
    }

    async fn delete_matter(&self, user_id: &str, matter_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.store.conn().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matters WHERE user_id = $1 AND matter_id = $2",
                &[&user_id, &matter_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

// ==================== MatterTaskStore ====================

#[async_trait]
impl MatterTaskStore for PgBackend {
    async fn list_matter_tasks(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterTaskRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks \
                 WHERE user_id = $1 AND matter_id = $2 \
                 ORDER BY created_at DESC",
                &[&user_id, &matter_id],
            )
            .await?;
        rows.into_iter()
            .map(|row| row_to_matter_task_record(&row))
            .collect()
    }

    async fn create_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterTaskParams,
    ) -> Result<MatterTaskRecord, DatabaseError> {
        let blocked_by = serde_json::to_value(
            input
                .blocked_by
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO matter_tasks \
                 (id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
                 RETURNING id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &matter_id,
                    &input.title,
                    &input.description,
                    &input.status.as_str(),
                    &input.assignee,
                    &input.due_at,
                    &blocked_by,
                ],
            )
            .await?;
        row_to_matter_task_record(&row)
    }

    async fn update_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
        input: &UpdateMatterTaskParams,
    ) -> Result<Option<MatterTaskRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let existing = conn
            .query_opt(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &task_id],
            )
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let existing = row_to_matter_task_record(&existing)?;

        let blocked_by = input.blocked_by.clone().unwrap_or(existing.blocked_by);
        let blocked_by = serde_json::to_value(
            blocked_by
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let merged_title = input.title.clone().unwrap_or(existing.title);
        let merged_description = input.description.clone().unwrap_or(existing.description);
        let merged_status = input.status.unwrap_or(existing.status);
        let merged_assignee = input.assignee.clone().unwrap_or(existing.assignee);
        let merged_due_at = input.due_at.unwrap_or(existing.due_at);

        let updated = conn
            .query_one(
                "UPDATE matter_tasks SET \
                    title = $4, \
                    description = $5, \
                    status = $6, \
                    assignee = $7, \
                    due_at = $8, \
                    blocked_by = $9, \
                    updated_at = NOW() \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3 \
                 RETURNING id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at",
                &[
                    &user_id,
                    &matter_id,
                    &task_id,
                    &merged_title,
                    &merged_description,
                    &merged_status.as_str(),
                    &merged_assignee,
                    &merged_due_at,
                    &blocked_by,
                ],
            )
            .await?;
        Ok(Some(row_to_matter_task_record(&updated)?))
    }

    async fn delete_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.store.conn().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_tasks WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &task_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

// ==================== MatterNoteStore ====================

#[async_trait]
impl MatterNoteStore for PgBackend {
    async fn list_matter_notes(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterNoteRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes \
                 WHERE user_id = $1 AND matter_id = $2 \
                 ORDER BY pinned DESC, created_at DESC",
                &[&user_id, &matter_id],
            )
            .await?;
        Ok(rows
            .iter()
            .map(row_to_matter_note_record)
            .collect::<Vec<_>>())
    }

    async fn create_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterNoteParams,
    ) -> Result<MatterNoteRecord, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO matter_notes (id, user_id, matter_id, author, body, pinned) \
                 VALUES ($1, $2, $3, $4, $5, $6) \
                 RETURNING id, user_id, matter_id, author, body, pinned, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &matter_id,
                    &input.author,
                    &input.body,
                    &input.pinned,
                ],
            )
            .await?;
        Ok(row_to_matter_note_record(&row))
    }

    async fn update_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
        input: &UpdateMatterNoteParams,
    ) -> Result<Option<MatterNoteRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let existing = conn
            .query_opt(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &note_id],
            )
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let existing = row_to_matter_note_record(&existing);

        let merged_author = input.author.clone().unwrap_or(existing.author);
        let merged_body = input.body.clone().unwrap_or(existing.body);
        let merged_pinned = input.pinned.unwrap_or(existing.pinned);

        let updated = conn
            .query_one(
                "UPDATE matter_notes SET \
                    author = $4, \
                    body = $5, \
                    pinned = $6, \
                    updated_at = NOW() \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3 \
                 RETURNING id, user_id, matter_id, author, body, pinned, created_at, updated_at",
                &[
                    &user_id,
                    &matter_id,
                    &note_id,
                    &merged_author,
                    &merged_body,
                    &merged_pinned,
                ],
            )
            .await?;
        Ok(Some(row_to_matter_note_record(&updated)))
    }

    async fn delete_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.store.conn().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_notes WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &note_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

// ==================== MatterDeadlineStore ====================

#[async_trait]
impl MatterDeadlineStore for PgBackend {
    async fn list_matter_deadlines(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterDeadlineRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at \
                 FROM matter_deadlines \
                 WHERE user_id = $1 AND matter_id = $2 \
                 ORDER BY due_at ASC, created_at ASC",
                &[&user_id, &matter_id],
            )
            .await?;
        rows.into_iter()
            .map(|row| row_to_matter_deadline_record(&row))
            .collect()
    }

    async fn get_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at \
                 FROM matter_deadlines \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &deadline_id],
            )
            .await?;
        row.map(|row| row_to_matter_deadline_record(&row))
            .transpose()
    }

    async fn create_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterDeadlineParams,
    ) -> Result<MatterDeadlineRecord, DatabaseError> {
        let reminder_days = serde_json::to_value(&input.reminder_days)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO matter_deadlines \
                 (id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                 RETURNING id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &matter_id,
                    &input.title,
                    &input.deadline_type.as_str(),
                    &input.due_at,
                    &input.completed_at,
                    &reminder_days,
                    &input.rule_ref,
                    &input.computed_from,
                    &input.task_id,
                ],
            )
            .await?;
        row_to_matter_deadline_record(&row)
    }

    async fn update_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
        input: &UpdateMatterDeadlineParams,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError> {
        let Some(existing) = self
            .get_matter_deadline(user_id, matter_id, deadline_id)
            .await?
        else {
            return Ok(None);
        };

        let merged_title = input.title.clone().unwrap_or(existing.title);
        let merged_deadline_type = input.deadline_type.unwrap_or(existing.deadline_type);
        let merged_due_at = input.due_at.unwrap_or(existing.due_at);
        let merged_completed_at = input.completed_at.unwrap_or(existing.completed_at);
        let merged_reminder_days = input
            .reminder_days
            .clone()
            .unwrap_or(existing.reminder_days);
        let merged_rule_ref = input.rule_ref.clone().unwrap_or(existing.rule_ref);
        let merged_computed_from = input.computed_from.unwrap_or(existing.computed_from);
        let merged_task_id = input.task_id.unwrap_or(existing.task_id);
        let reminder_days = serde_json::to_value(&merged_reminder_days)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "UPDATE matter_deadlines SET \
                    title = $4, \
                    deadline_type = $5, \
                    due_at = $6, \
                    completed_at = $7, \
                    reminder_days = $8, \
                    rule_ref = $9, \
                    computed_from = $10, \
                    task_id = $11, \
                    updated_at = NOW() \
                 WHERE user_id = $1 AND matter_id = $2 AND id = $3 \
                 RETURNING id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at",
                &[
                    &user_id,
                    &matter_id,
                    &deadline_id,
                    &merged_title,
                    &merged_deadline_type.as_str(),
                    &merged_due_at,
                    &merged_completed_at,
                    &reminder_days,
                    &merged_rule_ref,
                    &merged_computed_from,
                    &merged_task_id,
                ],
            )
            .await?;
        Ok(Some(row_to_matter_deadline_record(&row)?))
    }

    async fn delete_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.store.conn().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_deadlines WHERE user_id = $1 AND matter_id = $2 AND id = $3",
                &[&user_id, &matter_id, &deadline_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

// ==================== SettingsStore ====================

#[async_trait]
impl SettingsStore for PgBackend {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError> {
        self.store.get_setting(user_id, key).await
    }

    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError> {
        self.store.get_setting_full(user_id, key).await
    }

    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError> {
        self.store.set_setting(user_id, key, value).await
    }

    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError> {
        self.store.delete_setting(user_id, key).await
    }

    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError> {
        self.store.list_settings(user_id).await
    }

    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError> {
        self.store.get_all_settings(user_id).await
    }

    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError> {
        self.store.set_all_settings(user_id, settings).await
    }

    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError> {
        self.store.has_settings(user_id).await
    }
}

// ==================== WorkspaceStore ====================

#[async_trait]
impl WorkspaceStore for PgBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        self.repo.get_document_by_id(id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        self.repo
            .get_or_create_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        self.repo.update_document(id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        self.repo
            .delete_document_by_path(user_id, agent_id, path)
            .await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        self.repo.list_directory(user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        self.repo.list_all_paths(user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        self.repo.list_documents(user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        self.repo.delete_chunks(document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        self.repo
            .insert_chunk(document_id, chunk_index, content, embedding)
            .await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        self.repo.update_chunk_embedding(chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        self.repo
            .get_chunks_without_embeddings(user_id, agent_id, limit)
            .await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.repo
            .hybrid_search(user_id, agent_id, query, embedding, config)
            .await
    }
}
