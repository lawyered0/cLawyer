//! Database abstraction layer.
//!
//! Provides a backend-agnostic `Database` trait that unifies all persistence
//! operations. Two implementations exist behind feature flags:
//!
//! - `postgres` (default): Uses `deadpool-postgres` + `tokio-postgres`
//! - `libsql`: Uses libSQL (Turso's SQLite fork) for embedded/edge deployment
//!
//! The existing `Store`, `Repository`, `SecretsStore`, and `WasmToolStore`
//! types become thin wrappers that delegate to `Arc<dyn Database>`.

#[cfg(feature = "postgres")]
pub mod postgres;

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
pub mod libsql_migrations;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::BrokenTool;
use crate::agent::routine::{Routine, RoutineRun, RunStatus};
use crate::context::{ActionRecord, JobContext, JobState};
use crate::error::DatabaseError;
use crate::error::WorkspaceError;
use crate::history::{
    ConversationMessage, ConversationSummary, JobEventRecord, LlmCallRecord, SandboxJobRecord,
    SandboxJobSummary, SettingRow,
};
use crate::workspace::{MemoryChunk, MemoryDocument, WorkspaceEntry};
use crate::workspace::{SearchConfig, SearchResult};

/// Create a database backend from configuration, run migrations, and return it.
///
/// This is the shared helper for CLI commands and other call sites that need
/// a simple `Arc<dyn Database>` without retaining backend-specific handles
/// (e.g., `pg_pool` or `libsql_conn` for the secrets store). The main agent
/// startup in `main.rs` uses its own initialization block because it also
/// captures those backend-specific handles.
pub async fn connect_from_config(
    config: &crate::config::DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    match config.backend {
        #[cfg(feature = "libsql")]
        crate::config::DatabaseBackend::LibSql => {
            use secrecy::ExposeSecret as _;

            let default_path = crate::config::default_libsql_path();
            let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);

            let backend = if let Some(ref url) = config.libsql_url {
                let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                    DatabaseError::Pool(
                        "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                    )
                })?;
                libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            } else {
                libsql::LibSqlBackend::new_local(db_path)
                    .await
                    .map_err(|e| DatabaseError::Pool(e.to_string()))?
            };
            backend.run_migrations().await?;
            Ok(Arc::new(backend))
        }
        #[cfg(feature = "postgres")]
        _ => {
            let pg = postgres::PgBackend::new(config)
                .await
                .map_err(|e| DatabaseError::Pool(e.to_string()))?;
            pg.run_migrations().await?;
            Ok(Arc::new(pg))
        }
        #[cfg(not(feature = "postgres"))]
        _ => Err(DatabaseError::Pool(
            "No database backend available. Enable 'postgres' or 'libsql' feature.".to_string(),
        )),
    }
}

/// Role a party plays in a matter for conflict screening.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PartyRole {
    Client,
    Adverse,
    Related,
    Witness,
    Affiliate,
    Principal,
    OpposingCounsel,
}

impl PartyRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Adverse => "adverse",
            Self::Related => "related",
            Self::Witness => "witness",
            Self::Affiliate => "affiliate",
            Self::Principal => "principal",
            Self::OpposingCounsel => "opposing_counsel",
        }
    }

    pub fn base_db_role(self) -> &'static str {
        match self {
            Self::Affiliate | Self::Principal => Self::Related.as_str(),
            Self::OpposingCounsel => Self::Adverse.as_str(),
            _ => self.as_str(),
        }
    }

    pub fn role_detail(self) -> Option<&'static str> {
        match self {
            Self::Affiliate | Self::Principal | Self::OpposingCounsel => Some(self.as_str()),
            _ => None,
        }
    }

    pub fn from_db_role_columns(role: &str, role_detail: Option<&str>) -> Option<Self> {
        role_detail
            .and_then(Self::from_db_value)
            .or_else(|| Self::from_db_value(role))
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "client" => Some(Self::Client),
            "adverse" => Some(Self::Adverse),
            "related" => Some(Self::Related),
            "witness" => Some(Self::Witness),
            "affiliate" => Some(Self::Affiliate),
            "principal" => Some(Self::Principal),
            "opposing_counsel" => Some(Self::OpposingCounsel),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictClearanceStatus {
    #[default]
    Unreviewed,
    Clear,
    Waived,
    Declined,
}

impl ConflictClearanceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Clear => "clear",
            Self::Waived => "waived",
            Self::Declined => "declined",
        }
    }
}

impl From<ConflictDecision> for ConflictClearanceStatus {
    fn from(value: ConflictDecision) -> Self {
        match value {
            ConflictDecision::Clear => Self::Clear,
            ConflictDecision::Waived => Self::Waived,
            ConflictDecision::Declined => Self::Declined,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictClearanceSummary {
    pub decision: ConflictDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewing_attorney: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_hash: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Structured conflict match for legal intake and attorney review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictHit {
    pub party: String,
    pub role: PartyRole,
    pub matter_id: String,
    pub matter_status: String,
    pub matched_via: String,
    #[serde(default)]
    pub relationship_path: Vec<String>,
    #[serde(default)]
    pub clearance_status: ConflictClearanceStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_clearance: Option<ConflictClearanceSummary>,
}

/// Attorney decision after reviewing potential conflict hits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictDecision {
    Clear,
    Waived,
    Declined,
}

impl ConflictDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Waived => "waived",
            Self::Declined => "declined",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "clear" => Some(Self::Clear),
            "waived" => Some(Self::Waived),
            "declined" => Some(Self::Declined),
            _ => None,
        }
    }
}

/// Row persisted for legal conflict clearance history.
#[derive(Debug, Clone)]
pub struct ConflictClearanceRecord {
    pub matter_id: String,
    pub checked_by: String,
    pub cleared_by: Option<String>,
    pub decision: ConflictDecision,
    pub note: Option<String>,
    pub hits_json: serde_json::Value,
    pub hit_count: i32,
    pub reviewing_attorney: Option<String>,
    pub report_hash: Option<String>,
    pub signed_at: Option<DateTime<Utc>>,
}

/// Latest persisted conflict-clearance decision metadata for a matter.
#[derive(Debug, Clone)]
pub struct ConflictClearanceInfo {
    pub matter_id: String,
    pub checked_by: String,
    pub cleared_by: Option<String>,
    pub decision: ConflictDecision,
    pub note: Option<String>,
    pub hits_json: serde_json::Value,
    pub hit_count: i32,
    pub reviewing_attorney: Option<String>,
    pub report_hash: Option<String>,
    pub signed_at: Option<DateTime<Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterPartyRecord {
    pub id: Uuid,
    pub matter_id: String,
    pub party_id: Uuid,
    pub name: String,
    pub role: PartyRole,
    pub aliases: Vec<String>,
    pub notes: Option<String>,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertMatterPartyParams {
    pub name: String,
    pub role: PartyRole,
    pub aliases: Vec<String>,
    pub notes: Option<String>,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartyRelationshipRecord {
    pub id: Uuid,
    pub parent_party_id: Uuid,
    pub parent_name: String,
    pub child_party_id: Uuid,
    pub child_name: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreatePartyRelationshipParams {
    pub parent_party_id: Option<Uuid>,
    pub parent_name: Option<String>,
    pub child_party_id: Option<Uuid>,
    pub child_name: Option<String>,
    pub kind: String,
}

/// Role assigned to a gateway user identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Attorney,
    Staff,
    Viewer,
}

impl UserRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Attorney => "attorney",
            Self::Staff => "staff",
            Self::Viewer => "viewer",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "attorney" => Some(Self::Attorney),
            "staff" => Some(Self::Staff),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
}

/// Persisted user identity for gateway authorization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    pub id: String,
    pub display_name: String,
    pub role: UserRole,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Role a user has on a specific matter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterMemberRole {
    Owner,
    Collaborator,
    Viewer,
}

impl MatterMemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Collaborator => "collaborator",
            Self::Viewer => "viewer",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "owner" => Some(Self::Owner),
            "collaborator" => Some(Self::Collaborator),
            "viewer" => Some(Self::Viewer),
            _ => None,
        }
    }
}

/// Membership row linking a user to a matter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterMembershipRecord {
    pub id: Uuid,
    pub matter_owner_user_id: String,
    pub matter_id: String,
    pub member_user_id: String,
    pub role: MatterMemberRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertMatterMembershipParams {
    pub matter_owner_user_id: String,
    pub matter_id: String,
    pub member_user_id: String,
    pub role: MatterMemberRole,
}

/// Client entity type for conflict and matter tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientType {
    Individual,
    Entity,
}

impl ClientType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Individual => "individual",
            Self::Entity => "entity",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "individual" => Some(Self::Individual),
            "entity" => Some(Self::Entity),
            _ => None,
        }
    }
}

/// Matter lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterStatus {
    Intake,
    Active,
    Pending,
    Closed,
    Archived,
}

impl MatterStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "intake" => Some(Self::Intake),
            "active" => Some(Self::Active),
            "pending" => Some(Self::Pending),
            "closed" => Some(Self::Closed),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// Matter task state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterTaskStatus {
    Todo,
    InProgress,
    Done,
    Blocked,
    Cancelled,
}

impl MatterTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "todo" => Some(Self::Todo),
            "in_progress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            "blocked" => Some(Self::Blocked),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub name_normalized: String,
    pub client_type: ClientType,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateClientParams {
    pub name: String,
    pub client_type: ClientType,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateClientParams {
    pub name: Option<String>,
    pub client_type: Option<ClientType>,
    pub email: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub address: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterRecord {
    pub user_id: String,
    pub matter_id: String,
    pub client_id: Uuid,
    pub status: MatterStatus,
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

#[derive(Debug, Clone)]
pub struct UpsertMatterParams {
    pub matter_id: String,
    pub client_id: Uuid,
    pub status: MatterStatus,
    pub stage: Option<String>,
    pub practice_area: Option<String>,
    pub jurisdiction: Option<String>,
    pub opened_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub assigned_to: Vec<String>,
    pub custom_fields: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct UpdateMatterParams {
    pub client_id: Option<Uuid>,
    pub status: Option<MatterStatus>,
    pub stage: Option<Option<String>>,
    pub practice_area: Option<Option<String>>,
    pub jurisdiction: Option<Option<String>>,
    pub opened_at: Option<Option<DateTime<Utc>>>,
    pub closed_at: Option<Option<DateTime<Utc>>>,
    pub assigned_to: Option<Vec<String>>,
    pub custom_fields: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterTaskRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: MatterTaskStatus,
    pub assignee: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub blocked_by: Vec<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateMatterTaskParams {
    pub title: String,
    pub description: Option<String>,
    pub status: MatterTaskStatus,
    pub assignee: Option<String>,
    pub due_at: Option<DateTime<Utc>>,
    pub blocked_by: Vec<Uuid>,
}

#[derive(Debug, Clone)]
pub struct UpdateMatterTaskParams {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub status: Option<MatterTaskStatus>,
    pub assignee: Option<Option<String>>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub blocked_by: Option<Vec<Uuid>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterNoteRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub author: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateMatterNoteParams {
    pub author: String,
    pub body: String,
    pub pinned: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateMatterNoteParams {
    pub author: Option<String>,
    pub body: Option<String>,
    pub pinned: Option<bool>,
}

/// Matter deadline category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterDeadlineType {
    CourtDate,
    Filing,
    StatuteOfLimitations,
    ResponseDue,
    DiscoveryCutoff,
    Internal,
}

impl MatterDeadlineType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CourtDate => "court_date",
            Self::Filing => "filing",
            Self::StatuteOfLimitations => "statute_of_limitations",
            Self::ResponseDue => "response_due",
            Self::DiscoveryCutoff => "discovery_cutoff",
            Self::Internal => "internal",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "court_date" => Some(Self::CourtDate),
            "filing" => Some(Self::Filing),
            "statute_of_limitations" => Some(Self::StatuteOfLimitations),
            "response_due" => Some(Self::ResponseDue),
            "discovery_cutoff" => Some(Self::DiscoveryCutoff),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterDeadlineRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub title: String,
    pub deadline_type: MatterDeadlineType,
    pub due_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub reminder_days: Vec<i32>,
    pub rule_ref: Option<String>,
    pub computed_from: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateMatterDeadlineParams {
    pub title: String,
    pub deadline_type: MatterDeadlineType,
    pub due_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub reminder_days: Vec<i32>,
    pub rule_ref: Option<String>,
    pub computed_from: Option<Uuid>,
    pub task_id: Option<Uuid>,
}

#[derive(Debug, Clone)]
pub struct UpdateMatterDeadlineParams {
    pub title: Option<String>,
    pub deadline_type: Option<MatterDeadlineType>,
    pub due_at: Option<DateTime<Utc>>,
    pub completed_at: Option<Option<DateTime<Utc>>>,
    pub reminder_days: Option<Vec<i32>>,
    pub rule_ref: Option<Option<String>>,
    pub computed_from: Option<Option<Uuid>>,
    pub task_id: Option<Option<Uuid>>,
}

/// Matter document category for attorney workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatterDocumentCategory {
    Pleading,
    Correspondence,
    Contract,
    Filing,
    Evidence,
    Internal,
}

impl MatterDocumentCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pleading => "pleading",
            Self::Correspondence => "correspondence",
            Self::Contract => "contract",
            Self::Filing => "filing",
            Self::Evidence => "evidence",
            Self::Internal => "internal",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "pleading" => Some(Self::Pleading),
            "correspondence" => Some(Self::Correspondence),
            "contract" => Some(Self::Contract),
            "filing" => Some(Self::Filing),
            "evidence" => Some(Self::Evidence),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentReadinessState {
    #[default]
    Draft,
    CitationsPending,
    Verified,
    Waived,
    ReadyToFile,
}

impl DocumentReadinessState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::CitationsPending => "citations_pending",
            Self::Verified => "verified",
            Self::Waived => "waived",
            Self::ReadyToFile => "ready_to_file",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "citations_pending" => Some(Self::CitationsPending),
            "verified" => Some(Self::Verified),
            "waived" => Some(Self::Waived),
            "ready_to_file" => Some(Self::ReadyToFile),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterDocumentRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub memory_document_id: Uuid,
    pub path: String,
    pub display_name: String,
    pub category: MatterDocumentCategory,
    #[serde(default)]
    pub readiness_state: DocumentReadinessState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertMatterDocumentParams {
    pub memory_document_id: Uuid,
    pub path: String,
    pub display_name: String,
    pub category: MatterDocumentCategory,
    pub readiness_state: Option<DocumentReadinessState>,
}

#[derive(Debug, Clone)]
pub struct UpdateMatterDocumentParams {
    pub display_name: Option<String>,
    pub category: Option<MatterDocumentCategory>,
    pub readiness_state: Option<DocumentReadinessState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CitationVerificationStatus {
    Verified,
    Unverified,
    Ambiguous,
    Waived,
}

impl CitationVerificationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Ambiguous => "ambiguous",
            Self::Waived => "waived",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "verified" => Some(Self::Verified),
            "unverified" => Some(Self::Unverified),
            "ambiguous" => Some(Self::Ambiguous),
            "waived" => Some(Self::Waived),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationVerificationRunRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub matter_document_id: Uuid,
    pub provider: String,
    pub document_hash: String,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateCitationVerificationRunParams {
    pub matter_id: String,
    pub matter_document_id: Uuid,
    pub provider: String,
    pub document_hash: String,
    pub created_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationVerificationResultRecord {
    pub id: Uuid,
    pub run_id: Uuid,
    pub citation_text: String,
    pub normalized_citation: String,
    pub status: CitationVerificationStatus,
    pub provider_reference: Option<String>,
    pub provider_title: Option<String>,
    pub detail: Option<String>,
    pub waived_by: Option<String>,
    pub waiver_reason: Option<String>,
    pub waived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateCitationVerificationResultParams {
    pub citation_text: String,
    pub normalized_citation: String,
    pub status: CitationVerificationStatus,
    pub provider_reference: Option<String>,
    pub provider_title: Option<String>,
    pub detail: Option<String>,
    pub waived_by: Option<String>,
    pub waiver_reason: Option<String>,
    pub waived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentVersionRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_document_id: Uuid,
    pub version_number: i32,
    pub label: String,
    pub memory_document_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateDocumentVersionParams {
    pub matter_document_id: Uuid,
    pub label: String,
    pub memory_document_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentTemplateRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: Option<String>,
    pub name: String,
    pub body: String,
    pub variables_json: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertDocumentTemplateParams {
    pub matter_id: Option<String>,
    pub name: String,
    pub body: String,
    pub variables_json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct UpdateDocumentTemplateParams {
    pub name: Option<String>,
    pub body: Option<String>,
    pub variables_json: Option<serde_json::Value>,
}

/// Expense category for matter accounting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpenseCategory {
    FilingFee,
    Travel,
    Postage,
    Expert,
    Copying,
    CourtReporter,
    Other,
}

impl ExpenseCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FilingFee => "filing_fee",
            Self::Travel => "travel",
            Self::Postage => "postage",
            Self::Expert => "expert",
            Self::Copying => "copying",
            Self::CourtReporter => "court_reporter",
            Self::Other => "other",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "filing_fee" => Some(Self::FilingFee),
            "travel" => Some(Self::Travel),
            "postage" => Some(Self::Postage),
            "expert" => Some(Self::Expert),
            "copying" => Some(Self::Copying),
            "court_reporter" => Some(Self::CourtReporter),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeEntryRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub timekeeper: String,
    pub description: String,
    pub hours: Decimal,
    pub hourly_rate: Option<Decimal>,
    pub task_code: Option<String>,
    pub activity_code: Option<String>,
    pub resolved_rate: Option<Decimal>,
    pub rate_source: Option<BillingRateSource>,
    pub entry_date: NaiveDate,
    pub billable: bool,
    pub block_billing_flag: bool,
    pub block_billing_reason: Option<String>,
    pub billed_invoice_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateTimeEntryParams {
    pub timekeeper: String,
    pub description: String,
    pub hours: Decimal,
    pub hourly_rate: Option<Decimal>,
    pub task_code: Option<String>,
    pub activity_code: Option<String>,
    pub resolved_rate: Option<Decimal>,
    pub rate_source: Option<BillingRateSource>,
    pub entry_date: NaiveDate,
    pub billable: bool,
    pub block_billing_flag: bool,
    pub block_billing_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UpdateTimeEntryParams {
    pub timekeeper: Option<String>,
    pub description: Option<String>,
    pub hours: Option<Decimal>,
    pub hourly_rate: Option<Option<Decimal>>,
    pub task_code: Option<Option<String>>,
    pub activity_code: Option<Option<String>>,
    pub resolved_rate: Option<Option<Decimal>>,
    pub rate_source: Option<Option<BillingRateSource>>,
    pub entry_date: Option<NaiveDate>,
    pub billable: Option<bool>,
    pub block_billing_flag: Option<bool>,
    pub block_billing_reason: Option<Option<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingRateSource {
    MatterOverride,
    TimekeeperDefault,
    ManualOverride,
}

impl BillingRateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MatterOverride => "matter_override",
            Self::TimekeeperDefault => "timekeeper_default",
            Self::ManualOverride => "manual_override",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "matter_override" => Some(Self::MatterOverride),
            "timekeeper_default" => Some(Self::TimekeeperDefault),
            "manual_override" => Some(Self::ManualOverride),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRateScheduleRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: Option<String>,
    pub timekeeper: String,
    pub rate: Decimal,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateBillingRateScheduleParams {
    pub matter_id: Option<String>,
    pub timekeeper: String,
    pub rate: Decimal,
    pub effective_start: NaiveDate,
    pub effective_end: Option<NaiveDate>,
}

#[derive(Debug, Clone)]
pub struct UpdateBillingRateScheduleParams {
    pub matter_id: Option<Option<String>>,
    pub timekeeper: Option<String>,
    pub rate: Option<Decimal>,
    pub effective_start: Option<NaiveDate>,
    pub effective_end: Option<Option<NaiveDate>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseEntryRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub submitted_by: String,
    pub description: String,
    pub amount: Decimal,
    pub category: ExpenseCategory,
    pub entry_date: NaiveDate,
    pub receipt_path: Option<String>,
    pub billable: bool,
    pub billed_invoice_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateExpenseEntryParams {
    pub submitted_by: String,
    pub description: String,
    pub amount: Decimal,
    pub category: ExpenseCategory,
    pub entry_date: NaiveDate,
    pub receipt_path: Option<String>,
    pub billable: bool,
}

#[derive(Debug, Clone)]
pub struct UpdateExpenseEntryParams {
    pub submitted_by: Option<String>,
    pub description: Option<String>,
    pub amount: Option<Decimal>,
    pub category: Option<ExpenseCategory>,
    pub entry_date: Option<NaiveDate>,
    pub receipt_path: Option<Option<String>>,
    pub billable: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterTimeSummary {
    pub total_hours: Decimal,
    pub billable_hours: Decimal,
    pub unbilled_hours: Decimal,
    pub total_expenses: Decimal,
    pub billable_expenses: Decimal,
    pub unbilled_expenses: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceStatus {
    Draft,
    Sent,
    Paid,
    Void,
    WriteOff,
}

impl InvoiceStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Sent => "sent",
            Self::Paid => "paid",
            Self::Void => "void",
            Self::WriteOff => "write_off",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "sent" => Some(Self::Sent),
            "paid" => Some(Self::Paid),
            "void" => Some(Self::Void),
            "write_off" => Some(Self::WriteOff),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    pub invoice_number: String,
    pub status: InvoiceStatus,
    pub issued_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub subtotal: Decimal,
    pub tax: Decimal,
    pub total: Decimal,
    pub paid_amount: Decimal,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateInvoiceParams {
    pub matter_id: String,
    pub invoice_number: String,
    pub status: InvoiceStatus,
    pub issued_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub subtotal: Decimal,
    pub tax: Decimal,
    pub total: Decimal,
    pub paid_amount: Decimal,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLineItemRecord {
    pub id: Uuid,
    pub user_id: String,
    pub invoice_id: Uuid,
    pub description: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub amount: Decimal,
    pub time_entry_id: Option<Uuid>,
    pub expense_entry_id: Option<Uuid>,
    pub task_code: Option<String>,
    pub activity_code: Option<String>,
    pub timekeeper: Option<String>,
    pub resolved_rate: Option<Decimal>,
    pub rate_source: Option<BillingRateSource>,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateInvoiceLineItemParams {
    pub description: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
    pub amount: Decimal,
    pub time_entry_id: Option<Uuid>,
    pub expense_entry_id: Option<Uuid>,
    pub task_code: Option<String>,
    pub activity_code: Option<String>,
    pub timekeeper: Option<String>,
    pub resolved_rate: Option<Decimal>,
    pub rate_source: Option<BillingRateSource>,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLedgerEntryType {
    Deposit,
    Withdrawal,
    InvoicePayment,
    Refund,
    BankFee,
    FirmFunds,
}

impl TrustLedgerEntryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deposit => "deposit",
            Self::Withdrawal => "withdrawal",
            Self::InvoicePayment => "invoice_payment",
            Self::Refund => "refund",
            Self::BankFee => "bank_fee",
            Self::FirmFunds => "firm_funds",
        }
    }

    pub fn base_db_value(self) -> &'static str {
        match self {
            Self::BankFee => Self::Withdrawal.as_str(),
            Self::FirmFunds => Self::Deposit.as_str(),
            _ => self.as_str(),
        }
    }

    pub fn entry_detail(self) -> Option<&'static str> {
        match self {
            Self::BankFee | Self::FirmFunds => Some(self.as_str()),
            _ => None,
        }
    }

    pub fn from_db_columns(entry_type: &str, entry_detail: Option<&str>) -> Option<Self> {
        entry_detail
            .and_then(Self::from_db_value)
            .or_else(|| Self::from_db_value(entry_type))
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "deposit" => Some(Self::Deposit),
            "withdrawal" => Some(Self::Withdrawal),
            "invoice_payment" => Some(Self::InvoicePayment),
            "refund" => Some(Self::Refund),
            "bank_fee" => Some(Self::BankFee),
            "firm_funds" => Some(Self::FirmFunds),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustLedgerSource {
    #[default]
    Manual,
    StatementImport,
    InvoicePayment,
}

impl TrustLedgerSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::StatementImport => "statement_import",
            Self::InvoicePayment => "invoice_payment",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(Self::Manual),
            "statement_import" => Some(Self::StatementImport),
            "invoice_payment" => Some(Self::InvoicePayment),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustLedgerEntryRecord {
    pub id: Uuid,
    pub user_id: String,
    pub matter_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_account_id: Option<Uuid>,
    pub entry_type: TrustLedgerEntryType,
    pub amount: Decimal,
    pub delta: Decimal,
    pub balance_after: Decimal,
    pub description: String,
    pub reference_number: Option<String>,
    #[serde(default)]
    pub source: TrustLedgerSource,
    pub invoice_id: Option<Uuid>,
    pub recorded_by: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateTrustLedgerEntryParams {
    pub trust_account_id: Option<Uuid>,
    pub entry_type: TrustLedgerEntryType,
    pub amount: Decimal,
    /// Signed balance delta applied atomically by the backend.
    /// Positive values credit trust; negative values debit trust.
    pub delta: Decimal,
    pub description: String,
    pub reference_number: Option<String>,
    pub source: TrustLedgerSource,
    pub invoice_id: Option<Uuid>,
    pub recorded_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustAccountRecord {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub bank_name: Option<String>,
    pub account_number_last4: Option<String>,
    pub is_primary: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertTrustAccountParams {
    pub name: String,
    pub bank_name: Option<String>,
    pub account_number_last4: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStatementImportRecord {
    pub id: Uuid,
    pub user_id: String,
    pub trust_account_id: Uuid,
    pub statement_date: NaiveDate,
    pub starting_balance: Decimal,
    pub ending_balance: Decimal,
    pub imported_by: String,
    pub row_count: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateTrustStatementImportParams {
    pub trust_account_id: Uuid,
    pub statement_date: NaiveDate,
    pub starting_balance: Decimal,
    pub ending_balance: Decimal,
    pub imported_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustStatementLineRecord {
    pub id: Uuid,
    pub statement_import_id: Uuid,
    pub entry_date: NaiveDate,
    pub description: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub running_balance: Decimal,
    pub reference_number: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateTrustStatementLineParams {
    pub entry_date: NaiveDate,
    pub description: String,
    pub debit: Decimal,
    pub credit: Decimal,
    pub running_balance: Decimal,
    pub reference_number: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrustReconciliationStatus {
    #[default]
    Draft,
    SignedOff,
}

impl TrustReconciliationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::SignedOff => "signed_off",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "signed_off" => Some(Self::SignedOff),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustReconciliationRecord {
    pub id: Uuid,
    pub user_id: String,
    pub trust_account_id: Uuid,
    pub statement_import_id: Uuid,
    pub statement_ending_balance: Decimal,
    pub book_balance: Decimal,
    pub client_balance_total: Decimal,
    pub difference: Decimal,
    pub exceptions_json: serde_json::Value,
    pub status: TrustReconciliationStatus,
    pub signed_off_by: Option<String>,
    pub signed_off_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ComputeTrustReconciliationParams {
    pub trust_account_id: Uuid,
    pub statement_import_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct RecordInvoicePaymentParams {
    pub amount: Decimal,
    pub draw_from_trust: bool,
    pub recorded_by: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RecordInvoicePaymentResult {
    pub invoice: InvoiceRecord,
    pub trust_entry: Option<TrustLedgerEntryRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditSeverity {
    Info,
    Warn,
    Critical,
}

impl AuditSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Critical => "critical",
        }
    }

    pub fn from_db_value(value: &str) -> Option<Self> {
        match value {
            "info" => Some(Self::Info),
            "warn" => Some(Self::Warn),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEventRecord {
    pub id: Uuid,
    pub user_id: String,
    pub event_type: String,
    pub actor: String,
    pub matter_id: Option<String>,
    pub severity: AuditSeverity,
    pub details: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AppendAuditEventParams {
    pub event_type: String,
    pub actor: String,
    pub matter_id: Option<String>,
    pub severity: AuditSeverity,
    pub details: serde_json::Value,
}

#[derive(Debug, Clone, Default)]
pub struct AuditEventQuery {
    pub event_type: Option<String>,
    pub matter_id: Option<String>,
    pub severity: Option<AuditSeverity>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

/// Normalize names/text for conflict matching.
pub fn normalize_party_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_sep = true;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            out.push(' ');
            prev_sep = true;
        }
    }

    out.trim().to_string()
}

fn trigrams(text: &str) -> HashSet<String> {
    let normalized = normalize_party_name(text);
    if normalized.is_empty() {
        return HashSet::new();
    }
    let padded = format!("  {normalized}  ");
    let chars: Vec<char> = padded.chars().collect();
    let mut set = HashSet::new();
    if chars.len() < 3 {
        set.insert(padded);
        return set;
    }
    for i in 0..=(chars.len() - 3) {
        let tri = [chars[i], chars[i + 1], chars[i + 2]];
        set.insert(tri.iter().collect::<String>());
    }
    set
}

/// Jaccard-style trigram similarity in [0, 1].
pub fn trigram_similarity(a: &str, b: &str) -> f64 {
    let a_set = trigrams(a);
    let b_set = trigrams(b);
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

/// Build normalized candidate terms from free text for conflict matching.
pub fn conflict_terms_from_text(text: &str, active_matter: Option<&str>) -> Vec<String> {
    const MAX_TOKENS: usize = 32;
    const MAX_NGRAM: usize = 4;

    let normalized = normalize_party_name(text);
    let mut terms: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let tokens: Vec<&str> = normalized
        .split_whitespace()
        .filter(|token| token.len() >= 2)
        .take(MAX_TOKENS)
        .collect();

    for width in 1..=MAX_NGRAM {
        if width > tokens.len() {
            break;
        }
        for i in 0..=(tokens.len() - width) {
            let candidate = tokens[i..(i + width)].join(" ");
            if candidate.len() < 3 || !seen.insert(candidate.clone()) {
                continue;
            }
            terms.push(candidate);
        }
    }

    if let Some(matter) = active_matter {
        let normalized_matter = normalize_party_name(matter);
        if !normalized_matter.is_empty() && seen.insert(normalized_matter.clone()) {
            terms.push(normalized_matter);
        }
    }

    terms
}

// ==================== Sub-traits ====================
//
// Each sub-trait groups related persistence methods. The `Database` supertrait
// combines them all, so existing `Arc<dyn Database>` consumers keep working.
// Leaf consumers can depend on a specific sub-trait instead.

#[async_trait]
pub trait ConversationStore: Send + Sync {
    async fn create_conversation(
        &self,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<Uuid, DatabaseError>;
    async fn touch_conversation(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn add_conversation_message(
        &self,
        conversation_id: Uuid,
        role: &str,
        content: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn ensure_conversation(
        &self,
        id: Uuid,
        channel: &str,
        user_id: &str,
        thread_id: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn list_conversations_with_preview(
        &self,
        user_id: &str,
        channel: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn list_conversations_with_preview_for_matter(
        &self,
        user_id: &str,
        channel: &str,
        matter_id: &str,
        limit: i64,
    ) -> Result<Vec<ConversationSummary>, DatabaseError>;
    async fn get_or_create_assistant_conversation(
        &self,
        user_id: &str,
        channel: &str,
    ) -> Result<Uuid, DatabaseError>;
    async fn create_conversation_with_metadata(
        &self,
        channel: &str,
        user_id: &str,
        metadata: &serde_json::Value,
    ) -> Result<Uuid, DatabaseError>;
    async fn list_conversation_messages_paginated(
        &self,
        conversation_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<(Vec<ConversationMessage>, bool), DatabaseError>;
    async fn update_conversation_metadata_field(
        &self,
        id: Uuid,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn get_conversation_metadata(
        &self,
        id: Uuid,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn list_conversation_messages(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMessage>, DatabaseError>;
    async fn conversation_belongs_to_user(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn bind_conversation_to_matter(
        &self,
        conversation_id: Uuid,
        user_id: &str,
        matter_id: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_conversation_matter_id(
        &self,
        conversation_id: Uuid,
        user_id: &str,
    ) -> Result<Option<String>, DatabaseError>;
}

#[async_trait]
pub trait JobStore: Send + Sync {
    async fn save_job(&self, ctx: &JobContext) -> Result<(), DatabaseError>;
    async fn get_job(&self, id: Uuid) -> Result<Option<JobContext>, DatabaseError>;
    async fn update_job_status(
        &self,
        id: Uuid,
        status: JobState,
        failure_reason: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn mark_job_stuck(&self, id: Uuid) -> Result<(), DatabaseError>;
    async fn get_stuck_jobs(&self) -> Result<Vec<Uuid>, DatabaseError>;
    async fn save_action(&self, job_id: Uuid, action: &ActionRecord) -> Result<(), DatabaseError>;
    async fn get_job_actions(&self, job_id: Uuid) -> Result<Vec<ActionRecord>, DatabaseError>;
    async fn record_llm_call(&self, record: &LlmCallRecord<'_>) -> Result<Uuid, DatabaseError>;
    async fn save_estimation_snapshot(
        &self,
        job_id: Uuid,
        category: &str,
        tool_names: &[String],
        estimated_cost: Decimal,
        estimated_time_secs: i32,
        estimated_value: Decimal,
    ) -> Result<Uuid, DatabaseError>;
    async fn update_estimation_actuals(
        &self,
        id: Uuid,
        actual_cost: Decimal,
        actual_time_secs: i32,
        actual_value: Option<Decimal>,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SandboxStore: Send + Sync {
    async fn save_sandbox_job(&self, job: &SandboxJobRecord) -> Result<(), DatabaseError>;
    async fn get_sandbox_job(&self, id: Uuid) -> Result<Option<SandboxJobRecord>, DatabaseError>;
    async fn list_sandbox_jobs(&self) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn update_sandbox_job_status(
        &self,
        id: Uuid,
        status: &str,
        success: Option<bool>,
        message: Option<&str>,
        started_at: Option<DateTime<Utc>>,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<(), DatabaseError>;
    async fn cleanup_stale_sandbox_jobs(&self) -> Result<u64, DatabaseError>;
    async fn sandbox_job_summary(&self) -> Result<SandboxJobSummary, DatabaseError>;
    async fn list_sandbox_jobs_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<SandboxJobRecord>, DatabaseError>;
    async fn sandbox_job_summary_for_user(
        &self,
        user_id: &str,
    ) -> Result<SandboxJobSummary, DatabaseError>;
    async fn sandbox_job_belongs_to_user(
        &self,
        job_id: Uuid,
        user_id: &str,
    ) -> Result<bool, DatabaseError>;
    async fn update_sandbox_job_mode(&self, id: Uuid, mode: &str) -> Result<(), DatabaseError>;
    async fn get_sandbox_job_mode(&self, id: Uuid) -> Result<Option<String>, DatabaseError>;
    async fn save_job_event(
        &self,
        job_id: Uuid,
        event_type: &str,
        data: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn list_job_events(
        &self,
        job_id: Uuid,
        limit: Option<i64>,
    ) -> Result<Vec<JobEventRecord>, DatabaseError>;
}

#[async_trait]
pub trait RoutineStore: Send + Sync {
    async fn create_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn get_routine(&self, id: Uuid) -> Result<Option<Routine>, DatabaseError>;
    async fn get_routine_by_name(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<Routine>, DatabaseError>;
    async fn list_routines(&self, user_id: &str) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_event_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn list_due_cron_routines(&self) -> Result<Vec<Routine>, DatabaseError>;
    async fn update_routine(&self, routine: &Routine) -> Result<(), DatabaseError>;
    async fn update_routine_runtime(
        &self,
        id: Uuid,
        last_run_at: DateTime<Utc>,
        next_fire_at: Option<DateTime<Utc>>,
        run_count: u64,
        consecutive_failures: u32,
        state: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_routine(&self, id: Uuid) -> Result<bool, DatabaseError>;
    async fn create_routine_run(&self, run: &RoutineRun) -> Result<(), DatabaseError>;
    async fn complete_routine_run(
        &self,
        id: Uuid,
        status: RunStatus,
        result_summary: Option<&str>,
        tokens_used: Option<i32>,
    ) -> Result<(), DatabaseError>;
    async fn list_routine_runs(
        &self,
        routine_id: Uuid,
        limit: i64,
    ) -> Result<Vec<RoutineRun>, DatabaseError>;
    async fn count_running_routine_runs(&self, routine_id: Uuid) -> Result<i64, DatabaseError>;
    async fn link_routine_run_to_job(
        &self,
        run_id: Uuid,
        job_id: Uuid,
    ) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait ToolFailureStore: Send + Sync {
    async fn record_tool_failure(
        &self,
        tool_name: &str,
        error_message: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_broken_tools(&self, threshold: i32) -> Result<Vec<BrokenTool>, DatabaseError>;
    async fn mark_tool_repaired(&self, tool_name: &str) -> Result<(), DatabaseError>;
    async fn increment_repair_attempts(&self, tool_name: &str) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait LegalConflictStore: Send + Sync {
    async fn find_conflict_hits_for_names(
        &self,
        input_names: &[String],
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError>;
    async fn find_conflict_hits_for_text(
        &self,
        text: &str,
        active_matter: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConflictHit>, DatabaseError>;
    async fn seed_matter_parties(
        &self,
        matter_id: &str,
        client: &str,
        adversaries: &[String],
        opened_at: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn seed_conflict_entry(
        &self,
        matter_id: &str,
        canonical_name: &str,
        aliases: &[String],
        opened_at: Option<&str>,
    ) -> Result<(), DatabaseError>;
    async fn reset_conflict_graph(&self) -> Result<(), DatabaseError>;
    async fn upsert_party_aliases(
        &self,
        canonical_name: &str,
        aliases: &[String],
    ) -> Result<(), DatabaseError>;
    async fn record_conflict_clearance(
        &self,
        row: &ConflictClearanceRecord,
    ) -> Result<(), DatabaseError>;
    async fn latest_conflict_clearance(
        &self,
        matter_id: &str,
    ) -> Result<Option<ConflictClearanceInfo>, DatabaseError>;
    async fn list_matter_parties(
        &self,
        matter_id: &str,
    ) -> Result<Vec<MatterPartyRecord>, DatabaseError>;
    async fn upsert_matter_party(
        &self,
        matter_id: &str,
        input: &UpsertMatterPartyParams,
    ) -> Result<MatterPartyRecord, DatabaseError>;
    async fn list_matter_party_relationships(
        &self,
        matter_id: &str,
    ) -> Result<Vec<PartyRelationshipRecord>, DatabaseError>;
    async fn upsert_party_relationship(
        &self,
        input: &CreatePartyRelationshipParams,
    ) -> Result<PartyRelationshipRecord, DatabaseError>;
}

#[async_trait]
pub trait RbacStore: Send + Sync {
    /// Ensure a user row exists. Existing roles are preserved; only display_name is refreshed.
    async fn ensure_user_account(
        &self,
        user_id: &str,
        display_name: &str,
        default_role: UserRole,
    ) -> Result<UserRecord, DatabaseError>;
    async fn get_user_account(&self, user_id: &str) -> Result<Option<UserRecord>, DatabaseError>;
    async fn upsert_user_token_hash(
        &self,
        user_id: &str,
        token_hash: &str,
    ) -> Result<(), DatabaseError>;
    async fn get_user_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserRecord>, DatabaseError>;
    async fn upsert_matter_membership(
        &self,
        input: &UpsertMatterMembershipParams,
    ) -> Result<MatterMembershipRecord, DatabaseError>;
    async fn list_matter_memberships(
        &self,
        matter_owner_user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterMembershipRecord>, DatabaseError>;
    /// Check whether `requesting_user_id` has access to a matter.
    ///
    /// Returns `Some(MatterMemberRole::Owner)` immediately when the requester is
    /// the matter owner (no DB query). Otherwise queries `matter_memberships` and
    /// returns the stored role, or `None` if no membership row exists.
    async fn check_matter_access(
        &self,
        matter_owner_user_id: &str,
        matter_id: &str,
        requesting_user_id: &str,
    ) -> Result<Option<MatterMemberRole>, DatabaseError>;
    /// Delete a specific membership row. No-op if the row does not exist.
    async fn remove_matter_membership(
        &self,
        matter_owner_user_id: &str,
        matter_id: &str,
        member_user_id: &str,
    ) -> Result<(), DatabaseError>;
    /// Change a user's system role. Returns `None` if the user does not exist.
    async fn update_user_role(
        &self,
        user_id: &str,
        new_role: UserRole,
    ) -> Result<Option<UserRecord>, DatabaseError>;
    /// Set `is_active = false` for a user. No-op if the user does not exist.
    async fn deactivate_user(&self, user_id: &str) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait ClientStore: Send + Sync {
    async fn create_client(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError>;
    async fn upsert_client_by_normalized_name(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError>;
    async fn list_clients(
        &self,
        user_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClientRecord>, DatabaseError>;
    async fn get_client(
        &self,
        user_id: &str,
        client_id: Uuid,
    ) -> Result<Option<ClientRecord>, DatabaseError>;
    async fn update_client(
        &self,
        user_id: &str,
        client_id: Uuid,
        input: &UpdateClientParams,
    ) -> Result<Option<ClientRecord>, DatabaseError>;
    async fn delete_client(&self, user_id: &str, client_id: Uuid) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait MatterStore: Send + Sync {
    async fn upsert_matter(
        &self,
        user_id: &str,
        input: &UpsertMatterParams,
    ) -> Result<MatterRecord, DatabaseError>;
    async fn list_matters_db(&self, user_id: &str) -> Result<Vec<MatterRecord>, DatabaseError>;
    async fn get_matter_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Option<MatterRecord>, DatabaseError>;
    async fn update_matter(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpdateMatterParams,
    ) -> Result<Option<MatterRecord>, DatabaseError>;
    async fn delete_matter(&self, user_id: &str, matter_id: &str) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait MatterTaskStore: Send + Sync {
    async fn list_matter_tasks(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterTaskRecord>, DatabaseError>;
    async fn create_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterTaskParams,
    ) -> Result<MatterTaskRecord, DatabaseError>;
    async fn update_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
        input: &UpdateMatterTaskParams,
    ) -> Result<Option<MatterTaskRecord>, DatabaseError>;
    async fn delete_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait MatterNoteStore: Send + Sync {
    async fn list_matter_notes(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterNoteRecord>, DatabaseError>;
    async fn create_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterNoteParams,
    ) -> Result<MatterNoteRecord, DatabaseError>;
    async fn update_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
        input: &UpdateMatterNoteParams,
    ) -> Result<Option<MatterNoteRecord>, DatabaseError>;
    async fn delete_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait MatterDeadlineStore: Send + Sync {
    async fn list_matter_deadlines(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterDeadlineRecord>, DatabaseError>;
    async fn get_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError>;
    async fn create_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterDeadlineParams,
    ) -> Result<MatterDeadlineRecord, DatabaseError>;
    async fn update_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
        input: &UpdateMatterDeadlineParams,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError>;
    async fn delete_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait MatterDocumentStore: Send + Sync {
    async fn list_matter_documents_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterDocumentRecord>, DatabaseError>;
    async fn get_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError>;
    async fn get_matter_document_by_id(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError>;
    async fn upsert_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpsertMatterDocumentParams,
    ) -> Result<MatterDocumentRecord, DatabaseError>;
    async fn update_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
        input: &UpdateMatterDocumentParams,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError>;
    async fn delete_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait CitationVerificationStore: Send + Sync {
    async fn create_citation_verification_run(
        &self,
        user_id: &str,
        input: &CreateCitationVerificationRunParams,
        results: &[CreateCitationVerificationResultParams],
    ) -> Result<
        (
            CitationVerificationRunRecord,
            Vec<CitationVerificationResultRecord>,
        ),
        DatabaseError,
    >;
    async fn latest_citation_verification_run(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<CitationVerificationRunRecord>, DatabaseError>;
    async fn list_citation_verification_results(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Vec<CitationVerificationResultRecord>, DatabaseError>;
    async fn set_matter_document_readiness(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
        state: DocumentReadinessState,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError>;
}

#[async_trait]
pub trait DocumentVersionStore: Send + Sync {
    async fn list_document_versions(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Vec<DocumentVersionRecord>, DatabaseError>;
    async fn create_document_version(
        &self,
        user_id: &str,
        input: &CreateDocumentVersionParams,
    ) -> Result<DocumentVersionRecord, DatabaseError>;
}

#[async_trait]
pub trait DocumentTemplateStore: Send + Sync {
    async fn list_document_templates(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
    ) -> Result<Vec<DocumentTemplateRecord>, DatabaseError>;
    async fn get_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError>;
    async fn get_document_template_by_name(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
        name: &str,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError>;
    async fn upsert_document_template(
        &self,
        user_id: &str,
        input: &UpsertDocumentTemplateParams,
    ) -> Result<DocumentTemplateRecord, DatabaseError>;
    async fn update_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
        input: &UpdateDocumentTemplateParams,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError>;
    async fn delete_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
    ) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait TimeExpenseStore: Send + Sync {
    async fn list_time_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<TimeEntryRecord>, DatabaseError>;
    async fn get_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<Option<TimeEntryRecord>, DatabaseError>;
    async fn create_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateTimeEntryParams,
    ) -> Result<TimeEntryRecord, DatabaseError>;
    async fn update_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
        input: &UpdateTimeEntryParams,
    ) -> Result<Option<TimeEntryRecord>, DatabaseError>;
    async fn delete_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<bool, DatabaseError>;

    async fn list_expense_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<ExpenseEntryRecord>, DatabaseError>;
    async fn get_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<Option<ExpenseEntryRecord>, DatabaseError>;
    async fn create_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateExpenseEntryParams,
    ) -> Result<ExpenseEntryRecord, DatabaseError>;
    async fn update_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
        input: &UpdateExpenseEntryParams,
    ) -> Result<Option<ExpenseEntryRecord>, DatabaseError>;
    async fn delete_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<bool, DatabaseError>;

    async fn mark_time_entries_billed(
        &self,
        user_id: &str,
        entry_ids: &[Uuid],
        invoice_id: &str,
    ) -> Result<u64, DatabaseError>;
    async fn mark_expense_entries_billed(
        &self,
        user_id: &str,
        entry_ids: &[Uuid],
        invoice_id: &str,
    ) -> Result<u64, DatabaseError>;
    async fn matter_time_summary(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<MatterTimeSummary, DatabaseError>;
}

#[async_trait]
pub trait BillingRateStore: Send + Sync {
    async fn list_billing_rate_schedules(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
        timekeeper: Option<&str>,
    ) -> Result<Vec<BillingRateScheduleRecord>, DatabaseError>;
    async fn create_billing_rate_schedule(
        &self,
        user_id: &str,
        input: &CreateBillingRateScheduleParams,
    ) -> Result<BillingRateScheduleRecord, DatabaseError>;
    async fn update_billing_rate_schedule(
        &self,
        user_id: &str,
        schedule_id: Uuid,
        input: &UpdateBillingRateScheduleParams,
    ) -> Result<Option<BillingRateScheduleRecord>, DatabaseError>;
}

#[async_trait]
pub trait BillingStore: Send + Sync {
    async fn save_invoice_draft(
        &self,
        user_id: &str,
        invoice: &CreateInvoiceParams,
        line_items: &[CreateInvoiceLineItemParams],
    ) -> Result<(InvoiceRecord, Vec<InvoiceLineItemRecord>), DatabaseError>;
    async fn list_invoices(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
    ) -> Result<Vec<InvoiceRecord>, DatabaseError>;
    async fn get_invoice(
        &self,
        user_id: &str,
        invoice_id: Uuid,
    ) -> Result<Option<InvoiceRecord>, DatabaseError>;
    async fn list_invoice_line_items(
        &self,
        user_id: &str,
        invoice_id: Uuid,
    ) -> Result<Vec<InvoiceLineItemRecord>, DatabaseError>;
    async fn set_invoice_status(
        &self,
        user_id: &str,
        invoice_id: Uuid,
        status: InvoiceStatus,
        issued_date: Option<NaiveDate>,
    ) -> Result<Option<InvoiceRecord>, DatabaseError>;
    async fn finalize_invoice_atomic(
        &self,
        user_id: &str,
        invoice_id: Uuid,
    ) -> Result<Option<InvoiceRecord>, DatabaseError>;
    async fn apply_invoice_payment(
        &self,
        user_id: &str,
        invoice_id: Uuid,
        amount: Decimal,
    ) -> Result<Option<InvoiceRecord>, DatabaseError>;
    async fn record_invoice_payment(
        &self,
        user_id: &str,
        invoice_id: Uuid,
        input: &RecordInvoicePaymentParams,
    ) -> Result<Option<RecordInvoicePaymentResult>, DatabaseError>;
    async fn append_trust_ledger_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateTrustLedgerEntryParams,
    ) -> Result<TrustLedgerEntryRecord, DatabaseError>;
    async fn list_trust_ledger_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<TrustLedgerEntryRecord>, DatabaseError>;
    async fn current_trust_balance(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Decimal, DatabaseError>;
}

#[async_trait]
pub trait TrustAccountingStore: Send + Sync {
    async fn get_primary_trust_account(
        &self,
        user_id: &str,
    ) -> Result<Option<TrustAccountRecord>, DatabaseError>;
    async fn upsert_primary_trust_account(
        &self,
        user_id: &str,
        input: &UpsertTrustAccountParams,
    ) -> Result<TrustAccountRecord, DatabaseError>;
    async fn list_trust_ledger_entries_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Vec<TrustLedgerEntryRecord>, DatabaseError>;
    async fn current_trust_account_balance(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Decimal, DatabaseError>;
    async fn import_trust_statement(
        &self,
        user_id: &str,
        input: &CreateTrustStatementImportParams,
        lines: &[CreateTrustStatementLineParams],
    ) -> Result<(TrustStatementImportRecord, Vec<TrustStatementLineRecord>), DatabaseError>;
    async fn get_trust_statement_import(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Option<TrustStatementImportRecord>, DatabaseError>;
    async fn list_trust_statement_lines(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Vec<TrustStatementLineRecord>, DatabaseError>;
    async fn compute_trust_reconciliation(
        &self,
        user_id: &str,
        input: &ComputeTrustReconciliationParams,
    ) -> Result<TrustReconciliationRecord, DatabaseError>;
    async fn get_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError>;
    async fn latest_trust_reconciliation_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError>;
    async fn signoff_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
        signed_off_by: &str,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError>;
}

#[async_trait]
pub trait AuditEventStore: Send + Sync {
    async fn append_audit_event(
        &self,
        user_id: &str,
        input: &AppendAuditEventParams,
    ) -> Result<AuditEventRecord, DatabaseError>;
    async fn list_audit_events(
        &self,
        user_id: &str,
        query: &AuditEventQuery,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<AuditEventRecord>, DatabaseError>;
    async fn count_audit_events(
        &self,
        user_id: &str,
        query: &AuditEventQuery,
    ) -> Result<usize, DatabaseError>;
}

#[async_trait]
pub trait LegalRestoreStore: Send + Sync {
    async fn upsert_matter_task_record(&self, row: &MatterTaskRecord) -> Result<(), DatabaseError>;
    async fn upsert_matter_note_record(&self, row: &MatterNoteRecord) -> Result<(), DatabaseError>;
    async fn upsert_matter_deadline_record(
        &self,
        row: &MatterDeadlineRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_matter_document_record(
        &self,
        row: &MatterDocumentRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_document_version_record(
        &self,
        row: &DocumentVersionRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_time_entry_record(&self, row: &TimeEntryRecord) -> Result<(), DatabaseError>;
    async fn upsert_expense_entry_record(
        &self,
        row: &ExpenseEntryRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_invoice_record(&self, row: &InvoiceRecord) -> Result<(), DatabaseError>;
    async fn upsert_invoice_line_item_record(
        &self,
        row: &InvoiceLineItemRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_trust_ledger_entry_record(
        &self,
        row: &TrustLedgerEntryRecord,
    ) -> Result<(), DatabaseError>;
    async fn upsert_audit_event_record(&self, row: &AuditEventRecord) -> Result<(), DatabaseError>;
}

#[async_trait]
pub trait SettingsStore: Send + Sync {
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, DatabaseError>;
    async fn get_setting_full(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<SettingRow>, DatabaseError>;
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), DatabaseError>;
    async fn delete_setting(&self, user_id: &str, key: &str) -> Result<bool, DatabaseError>;
    async fn list_settings(&self, user_id: &str) -> Result<Vec<SettingRow>, DatabaseError>;
    async fn get_all_settings(
        &self,
        user_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, DatabaseError>;
    async fn set_all_settings(
        &self,
        user_id: &str,
        settings: &HashMap<String, serde_json::Value>,
    ) -> Result<(), DatabaseError>;
    async fn has_settings(&self, user_id: &str) -> Result<bool, DatabaseError>;
}

#[async_trait]
pub trait WorkspaceStore: Send + Sync {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError>;
    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError>;
    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError>;
    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError>;
    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError>;
    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError>;
    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError>;
    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError>;
    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError>;
    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;
    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError>;
    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError>;
}

/// Backend-agnostic database supertrait.
///
/// Combines all sub-traits into one. Existing `Arc<dyn Database>` consumers
/// continue to work; leaf consumers can depend on a specific sub-trait instead.
#[async_trait]
pub trait Database:
    ConversationStore
    + JobStore
    + SandboxStore
    + RoutineStore
    + ToolFailureStore
    + LegalConflictStore
    + RbacStore
    + ClientStore
    + MatterStore
    + MatterTaskStore
    + MatterNoteStore
    + MatterDeadlineStore
    + MatterDocumentStore
    + CitationVerificationStore
    + DocumentVersionStore
    + DocumentTemplateStore
    + TimeExpenseStore
    + BillingRateStore
    + BillingStore
    + TrustAccountingStore
    + AuditEventStore
    + LegalRestoreStore
    + SettingsStore
    + WorkspaceStore
    + Send
    + Sync
{
    /// Run schema migrations for this backend.
    async fn run_migrations(&self) -> Result<(), DatabaseError>;
}
