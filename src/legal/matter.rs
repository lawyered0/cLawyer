use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::config::LegalConfig;
use crate::db::{ClientType, CreateClientParams, Database, MatterStatus, UpsertMatterParams};
use crate::error::WorkspaceError;
use crate::legal::policy::sanitize_matter_id;
use crate::workspace::Workspace;

const CONFLICT_CACHE_REFRESH_WINDOW: Duration = Duration::from_secs(30);
// Short single-token aliases (for example "corp") produce high false-positive
// rates in free-form text, so we require at least 4 characters for those terms.
const MIN_ALIAS_SINGLE_TOKEN_LEN: usize = 4;
const MATTER_PROMPT_LIST_MAX_ITEMS: usize = 8;
const MATTER_PROMPT_FIELD_MAX_CHARS: usize = 160;
const MATTER_PROMPT_LIST_ITEM_MAX_CHARS: usize = 96;
pub const GLOBAL_CONFLICT_GRAPH_MATTER_ID: &str = "__global_conflicts__";
const REINDEX_WARNING_LIMIT: usize = 50;

#[derive(Debug, Clone)]
struct ConflictEntry {
    canonical_name: String,
    terms: Vec<String>,
    aliases: Vec<String>,
}

#[derive(Debug, Default)]
struct ConflictCacheState {
    entries: Vec<ConflictEntry>,
    generation: u64,
    refreshed_at: Option<Instant>,
    ready: bool,
}

#[derive(Debug, Clone)]
struct DbConflictCacheEntry {
    conflict: Option<String>,
    refreshed_at: Instant,
}

#[derive(Debug, Default)]
struct DbConflictCacheState {
    entries: HashMap<String, DbConflictCacheEntry>,
    generation: u64,
}

static CONFLICT_CACHE: LazyLock<Mutex<ConflictCacheState>> =
    LazyLock::new(|| Mutex::new(ConflictCacheState::default()));
static DB_CONFLICT_CACHE: LazyLock<Mutex<DbConflictCacheState>> =
    LazyLock::new(|| Mutex::new(DbConflictCacheState::default()));
static CONFLICT_REINDEX_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));
static CONFLICT_CACHE_GENERATION: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static CONFLICT_CACHE_REFRESH_COUNT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterMetadata {
    pub matter_id: String,
    pub client: String,
    pub team: Vec<String>,
    pub confidentiality: String,
    pub adversaries: Vec<String>,
    pub retention: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jurisdiction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub practice_area: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMatterPromptContext {
    pub matter_id: String,
    pub client: String,
    pub confidentiality: String,
    pub retention: String,
    pub team: Vec<String>,
    pub adversaries: Vec<String>,
    pub jurisdiction: Option<String>,
    pub practice_area: Option<String>,
    pub opened_at: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ConflictGraphReindexReport {
    pub scanned_matters: usize,
    pub seeded_matters: usize,
    pub skipped_matters: usize,
    pub global_conflicts_seeded: usize,
    pub global_aliases_seeded: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct MatterWorkspaceReindexReport {
    pub scanned_matters: usize,
    pub upserted_matters: usize,
    pub skipped_matters: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct ReindexMatterData {
    metadata: MatterMetadata,
    status: MatterStatus,
    stage: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyMatterMetadata {
    #[serde(default)]
    matter_id: Option<String>,
    #[serde(default)]
    client: Option<String>,
    #[serde(default)]
    team: Vec<String>,
    #[serde(default)]
    confidentiality: Option<String>,
    #[serde(default)]
    adversaries: Vec<String>,
    #[serde(default)]
    retention: Option<String>,
    #[serde(default)]
    jurisdiction: Option<String>,
    #[serde(default)]
    practice_area: Option<String>,
    #[serde(default)]
    opened_at: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    stage: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatterMetadataValidationError {
    Missing { path: String },
    Invalid { path: String, reason: String },
    Storage { path: String, reason: String },
}

impl fmt::Display for MatterMetadataValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => {
                write!(f, "missing required matter metadata at '{}'", path)
            }
            Self::Invalid { path, reason } => write!(f, "{} in '{}'", reason, path),
            Self::Storage { path, reason } => {
                write!(f, "failed to read matter metadata '{}': {}", path, reason)
            }
        }
    }
}

impl MatterMetadata {
    pub fn validate_required_fields(&self) -> Result<(), String> {
        if self.matter_id.trim().is_empty() {
            return Err("matter_id is required".to_string());
        }
        if self.client.trim().is_empty() {
            return Err("client is required".to_string());
        }
        if self.confidentiality.trim().is_empty() {
            return Err("confidentiality is required".to_string());
        }
        if self.retention.trim().is_empty() {
            return Err("retention is required".to_string());
        }
        Ok(())
    }
}

pub fn matter_prefix(config: &LegalConfig, matter_id: &str) -> String {
    let root = config.matter_root.trim_matches('/');
    let id = sanitize_matter_id(matter_id);
    format!("{root}/{id}")
}

pub fn matter_metadata_path(config: &LegalConfig, matter_id: &str) -> String {
    format!("{}/matter.yaml", matter_prefix(config, matter_id))
}

pub fn matter_metadata_path_for_root(matter_root: &str, matter_id: &str) -> String {
    let root = matter_root.trim_matches('/');
    let id = sanitize_matter_id(matter_id);
    format!("{root}/{id}/matter.yaml")
}

pub async fn read_matter_metadata_for_root(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Result<MatterMetadata, MatterMetadataValidationError> {
    let metadata_path = matter_metadata_path_for_root(matter_root, matter_id);
    let doc = workspace
        .read(&metadata_path)
        .await
        .map_err(|err| match err {
            WorkspaceError::DocumentNotFound { .. } => MatterMetadataValidationError::Missing {
                path: metadata_path.clone(),
            },
            other => MatterMetadataValidationError::Storage {
                path: metadata_path.clone(),
                reason: other.to_string(),
            },
        })?;

    let metadata: MatterMetadata =
        serde_yml::from_str(&doc.content).map_err(|e| MatterMetadataValidationError::Invalid {
            path: metadata_path.clone(),
            reason: format!("invalid matter.yaml format: {}", e),
        })?;

    metadata.validate_required_fields().map_err(|reason| {
        MatterMetadataValidationError::Invalid {
            path: metadata_path.clone(),
            reason,
        }
    })?;

    let expected = sanitize_matter_id(matter_id);
    if metadata.matter_id != expected {
        return Err(MatterMetadataValidationError::Invalid {
            path: metadata_path,
            reason: format!(
                "matter.yaml mismatch: expected matter_id '{}', got '{}'",
                expected, metadata.matter_id
            ),
        });
    }

    Ok(metadata)
}

fn sanitize_prompt_field(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut seen_non_ws = false;
    let mut pending_space = false;
    let mut count = 0usize;

    for ch in value.chars() {
        let normalized = if ch.is_control() {
            if matches!(ch, '\n' | '\r' | '\t') {
                ' '
            } else {
                continue;
            }
        } else {
            ch
        };

        if normalized.is_whitespace() {
            if seen_non_ws {
                pending_space = true;
            }
            continue;
        }

        if pending_space {
            if count >= max_chars {
                break;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }

        if count >= max_chars {
            break;
        }
        out.push(normalized);
        count += 1;
        seen_non_ws = true;
    }

    out
}

fn sanitize_prompt_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| sanitize_prompt_field(value, MATTER_PROMPT_LIST_ITEM_MAX_CHARS))
        .filter(|value| !value.is_empty())
        .take(MATTER_PROMPT_LIST_MAX_ITEMS)
        .collect()
}

/// Build active matter context fields suitable for inclusion in the legal
/// system prompt as untrusted data.
pub async fn load_active_matter_prompt_context(
    workspace: &Workspace,
    config: &LegalConfig,
) -> Result<Option<ActiveMatterPromptContext>, MatterMetadataValidationError> {
    if !config.enabled {
        return Ok(None);
    }

    let active_matter = match config.active_matter.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Ok(None),
    };

    let metadata =
        read_matter_metadata_for_root(workspace, &config.matter_root, active_matter).await?;
    let sanitized_matter_id = sanitize_matter_id(&metadata.matter_id);
    let matter_id = if sanitized_matter_id.is_empty() {
        sanitize_prompt_field(active_matter, MATTER_PROMPT_FIELD_MAX_CHARS)
    } else {
        sanitize_prompt_field(&sanitized_matter_id, MATTER_PROMPT_FIELD_MAX_CHARS)
    };

    Ok(Some(ActiveMatterPromptContext {
        matter_id,
        client: sanitize_prompt_field(&metadata.client, MATTER_PROMPT_FIELD_MAX_CHARS),
        confidentiality: sanitize_prompt_field(
            &metadata.confidentiality,
            MATTER_PROMPT_FIELD_MAX_CHARS,
        ),
        retention: sanitize_prompt_field(&metadata.retention, MATTER_PROMPT_FIELD_MAX_CHARS),
        team: sanitize_prompt_list(&metadata.team),
        adversaries: sanitize_prompt_list(&metadata.adversaries),
        jurisdiction: metadata
            .jurisdiction
            .as_deref()
            .map(|value| sanitize_prompt_field(value, MATTER_PROMPT_FIELD_MAX_CHARS))
            .filter(|value| !value.is_empty()),
        practice_area: metadata
            .practice_area
            .as_deref()
            .map(|value| sanitize_prompt_field(value, MATTER_PROMPT_FIELD_MAX_CHARS))
            .filter(|value| !value.is_empty()),
        opened_at: metadata
            .opened_at
            .as_deref()
            .map(|value| sanitize_prompt_field(value, MATTER_PROMPT_FIELD_MAX_CHARS))
            .filter(|value| !value.is_empty()),
    }))
}

/// Validate `matter.yaml` for the active matter context.
pub async fn validate_active_matter_metadata(
    workspace: &Workspace,
    config: &LegalConfig,
) -> Result<(), String> {
    if !config.enabled {
        return Ok(());
    }

    let matter_id = match config.active_matter.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Ok(()),
    };

    read_matter_metadata_for_root(workspace, &config.matter_root, matter_id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Seed legal workspace scaffolding if legal mode is enabled.
pub async fn seed_legal_workspace(
    workspace: &Workspace,
    config: &LegalConfig,
) -> Result<(), WorkspaceError> {
    if !config.enabled {
        return Ok(());
    }

    // Seed conflict list template.
    match workspace.read("conflicts.json").await {
        Ok(_) => {}
        Err(WorkspaceError::DocumentNotFound { .. }) => {
            workspace
                .write(
                    "conflicts.json",
                    "[\n  {\n    \"name\": \"Example Adverse Party\",\n    \"aliases\": [\"Example Co\"]\n  }\n]\n",
                )
                .await?;
        }
        Err(e) => return Err(e),
    }

    let root_seeds = [
        (
            "AGENTS.md".to_string(),
            "# cLawyer Legal Guardrails\n\n\
             - Treat all `matters/*` files as confidential by default.\n\
             - Require source citations for factual/legal assertions.\n\
             - If evidence is missing, state `insufficient evidence`.\n\
             - Keep facts and analysis in separate sections.\n\
             - Do not export matter data externally without explicit approval.\n"
                .to_string(),
        ),
        (
            "legal/CITATION_STYLE_GUIDE.md".to_string(),
            "# Citation Style Guide\n\n\
             Use short source references after each supported statement:\n\
             - `[doc:<name> page:<n> section:<heading>]`\n\
             - `[authority:<name> §<section>]`\n\
             - For uncertain support, mark: `insufficient evidence`.\n"
                .to_string(),
        ),
        (
            "legal/CONFIDENTIALITY_NOTES.md".to_string(),
            "# Confidentiality Handling Notes\n\n\
             - Matter files are privileged by default.\n\
             - Redact SSNs, financial account numbers, and government IDs in exports.\n\
             - Confirm approvals before external transmission or publication.\n"
                .to_string(),
        ),
        (
            format!(
                "{}/_template/matter.yaml",
                config.matter_root.trim_matches('/')
            ),
            "matter_id: example-matter\n\
             client: Example Client\n\
             team:\n\
               - Lead Counsel\n\
             confidentiality: attorney-client-privileged\n\
             adversaries:\n\
               - Example Adverse Party\n\
             retention: follow-firm-policy\n"
                .to_string(),
        ),
    ];

    for (path, content) in root_seeds {
        match workspace.read(&path).await {
            Ok(_) => {}
            Err(WorkspaceError::DocumentNotFound { .. }) => {
                workspace.write(&path, &content).await?;
            }
            Err(e) => return Err(e),
        }
    }

    let matter_id = match config.active_matter.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Ok(()),
    };

    let prefix = matter_prefix(config, matter_id);
    let metadata_path = format!("{prefix}/matter.yaml");
    let metadata = MatterMetadata {
        matter_id: sanitize_matter_id(matter_id),
        client: "TBD Client".to_string(),
        team: vec!["Lead Counsel".to_string()],
        confidentiality: "attorney-client-privileged".to_string(),
        adversaries: Vec::new(),
        retention: "follow-firm-policy".to_string(),
        jurisdiction: None,
        practice_area: None,
        opened_at: None,
    };
    let matter_yaml =
        serde_yml::to_string(&metadata).map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("failed to serialize matter metadata: {}", e),
        })?;

    let seeds = [
        (
            format!("{prefix}/README.md"),
            format!(
                "# Matter {}\n\nThis matter workspace is scoped for confidential legal work.\n\n\
                 Files in this tree are treated as privileged by default.\n\n\
                 ## Suggested Workflow\n\n\
                 1. Intake and conflicts\n\
                 2. Facts and chronology\n\
                 3. Research and authority synthesis\n\
                 4. Drafting and review\n\
                 5. Filing and follow-up\n",
                sanitize_matter_id(matter_id)
            ),
        ),
        (
            metadata_path.clone(),
            format!(
                "# Matter metadata schema\n# Required: matter_id, client, confidentiality, retention\n{}",
                matter_yaml
            ),
        ),
        (
            format!("{prefix}/workflows/intake_checklist.md"),
            "# Intake Checklist\n\n- [ ] Confirm engagement and scope\n- [ ] Confirm client contact and billing details\n- [ ] Run conflict check and document result\n- [ ] Capture key deadlines and court dates\n- [ ] Identify required initial filings or responses\n".to_string(),
        ),
        (
            format!("{prefix}/workflows/review_and_filing_checklist.md"),
            "# Review and Filing Checklist\n\n- [ ] Separate facts from analysis in final draft\n- [ ] Verify citation format coverage for factual/legal assertions\n- [ ] Confirm privilege/confidentiality review complete\n- [ ] Final QA pass and attorney approval recorded\n- [ ] Filing/service steps completed and logged\n".to_string(),
        ),
        (
            format!("{prefix}/deadlines/calendar.md"),
            "# Deadlines and Hearings\n\n| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/facts/key_facts.md"),
            "# Key Facts Log\n\n| Fact | Source | Confidence | Notes |\n|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/research/authority_table.md"),
            "# Authority Table\n\n| Authority | Holding / Principle | Relevance | Risk / Limit | Citation |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/discovery/request_tracker.md"),
            "# Discovery Request Tracker\n\n| Request / Topic | Served / Received | Response Due | Status | Notes |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/communications/contact_log.md"),
            "# Communications Log\n\n| Date | With | Channel | Summary | Follow-up |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/templates/research_memo.md"),
            "# Research Memo Template\n\n## Question Presented\n\n## Brief Answer\n\n## Facts\n- [Doc/page]\n\n## Analysis\n\n## Authorities\n- [citation]\n\n## Uncertainty/Risk\n".to_string(),
        ),
        (
            format!("{prefix}/templates/chronology.md"),
            "# Chronology\n\n| Date | Event | Source |\n|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/templates/contract_issues.md"),
            "# Contract Issue List\n\n## Clause\n\n## Risk\n\n## Recommendation\n\n## Source\n".to_string(),
        ),
        (
            format!("{prefix}/templates/discovery_plan.md"),
            "# Discovery Plan\n\n## Custodians\n\n## Data Sources\n\n## Requests\n\n## Objections/Risks\n\n## Source Traceability\n".to_string(),
        ),
        (
            format!("{prefix}/templates/research_synthesis.md"),
            "# Research Synthesis\n\n## Question Presented\n\n## Authorities Reviewed\n\n## Facts (Cited)\n\n## Analysis\n\n## Uncertainty/Risk\n".to_string(),
        ),
        (
            format!("{prefix}/templates/legal_memo.md"),
            "# Legal Memo\n\n## Issue\n\n## Brief Answer\n\n## Facts (Cited)\n\n## Analysis\n\n## Conclusion\n\n## Uncertainty/Risk\n".to_string(),
        ),
    ];

    for (path, content) in seeds {
        match workspace.read(&path).await {
            Ok(_) => {}
            Err(WorkspaceError::DocumentNotFound { .. }) => {
                workspace.write(&path, &content).await?;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

fn push_reindex_warning(report: &mut ConflictGraphReindexReport, message: String) {
    if report.warnings.len() < REINDEX_WARNING_LIMIT {
        report.warnings.push(message);
    }
}

fn push_matter_reindex_warning(report: &mut MatterWorkspaceReindexReport, message: String) {
    if report.warnings.len() < REINDEX_WARNING_LIMIT {
        report.warnings.push(message);
    }
}

fn parse_optional_opened_at_ts(raw: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    if let Ok(date) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") {
        let dt = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid opened_at date".to_string())?;
        return Ok(Some(dt.and_utc()));
    }

    if let Ok(dt) = DateTime::parse_from_rfc3339(raw) {
        return Ok(Some(dt.with_timezone(&Utc)));
    }

    Err(format!("invalid opened_at timestamp '{}'", raw))
}

fn parse_matter_status_hint(raw: Option<&str>) -> MatterStatus {
    match raw
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "intake" => MatterStatus::Intake,
        "pending" => MatterStatus::Pending,
        "closed" => MatterStatus::Closed,
        "archived" => MatterStatus::Archived,
        _ => MatterStatus::Active,
    }
}

fn parse_optional_trimmed(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

async fn load_reindex_matter_data_for_root(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Result<ReindexMatterData, String> {
    match read_matter_metadata_for_root(workspace, matter_root, matter_id).await {
        Ok(metadata) => {
            return Ok(ReindexMatterData {
                metadata,
                status: MatterStatus::Active,
                stage: None,
            });
        }
        Err(MatterMetadataValidationError::Missing { .. }) => {}
        Err(err) => return Err(err.to_string()),
    }

    let metadata_path = format!(
        "{}/{}/metadata.json",
        matter_root.trim_matches('/'),
        matter_id
    );
    let legacy_doc = workspace
        .read(&metadata_path)
        .await
        .map_err(|err| match err {
            WorkspaceError::DocumentNotFound { .. } => {
                format!(
                    "missing required matter metadata at '{}' (matter.yaml or metadata.json)",
                    metadata_path
                )
            }
            other => format!("failed to read '{}': {}", metadata_path, other),
        })?;

    let legacy: LegacyMatterMetadata =
        serde_json::from_str(&legacy_doc.content).map_err(|err| {
            format!(
                "invalid metadata.json format in '{}': {}",
                metadata_path, err
            )
        })?;
    let matter_id_raw = legacy.matter_id.unwrap_or_else(|| matter_id.to_string());
    let normalized_id = sanitize_matter_id(&matter_id_raw);
    if normalized_id != sanitize_matter_id(matter_id) {
        return Err(format!(
            "metadata.json mismatch: expected matter_id '{}', got '{}'",
            sanitize_matter_id(matter_id),
            matter_id_raw
        ));
    }

    let client = parse_optional_trimmed(legacy.client)
        .ok_or_else(|| format!("client is required in '{}'", metadata_path))?;
    let confidentiality = parse_optional_trimmed(legacy.confidentiality)
        .unwrap_or_else(|| "attorney-client privilege".to_string());
    let retention = parse_optional_trimmed(legacy.retention)
        .unwrap_or_else(|| "standard legal hold".to_string());

    let metadata = MatterMetadata {
        matter_id: sanitize_matter_id(matter_id),
        client,
        team: legacy
            .team
            .into_iter()
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        confidentiality,
        adversaries: legacy
            .adversaries
            .into_iter()
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect(),
        retention,
        jurisdiction: parse_optional_trimmed(legacy.jurisdiction),
        practice_area: parse_optional_trimmed(legacy.practice_area),
        opened_at: parse_optional_trimmed(legacy.opened_at),
    };

    metadata
        .validate_required_fields()
        .map_err(|err| format!("invalid '{}': {}", metadata_path, err))?;

    Ok(ReindexMatterData {
        metadata,
        status: parse_matter_status_hint(legacy.status.as_deref()),
        stage: parse_optional_trimmed(legacy.stage),
    })
}

/// Rebuild DB-backed client/matter rows from workspace metadata.
///
/// Reads `matter.yaml` first and falls back to `metadata.json` for older
/// workspaces that have not been migrated yet.
pub async fn reindex_matters_from_workspace(
    workspace: &Workspace,
    store: &std::sync::Arc<dyn Database>,
    config: &LegalConfig,
    user_id: &str,
) -> Result<MatterWorkspaceReindexReport, String> {
    let matter_root = config.matter_root.trim_matches('/');
    if matter_root.is_empty() {
        return Err("legal matter root is empty after normalization".to_string());
    }

    let matter_entries = workspace
        .list(matter_root)
        .await
        .map_err(|err| format!("failed to list matter root '{matter_root}': {err}"))?;
    let mut report = MatterWorkspaceReindexReport::default();

    for entry in matter_entries
        .into_iter()
        .filter(|entry| entry.is_directory)
    {
        report.scanned_matters += 1;
        let raw_id = entry.path.rsplit('/').next().unwrap_or_default();
        let matter_id = sanitize_matter_id(raw_id);
        if matter_id.is_empty() || matter_id == "_template" {
            report.skipped_matters += 1;
            push_matter_reindex_warning(
                &mut report,
                format!(
                    "skipped matter directory '{}' (invalid matter id)",
                    entry.path
                ),
            );
            continue;
        }

        let data = match load_reindex_matter_data_for_root(workspace, matter_root, &matter_id).await
        {
            Ok(data) => data,
            Err(err) => {
                report.skipped_matters += 1;
                push_matter_reindex_warning(
                    &mut report,
                    format!(
                        "skipped matter '{}' due to invalid metadata: {}",
                        matter_id, err
                    ),
                );
                continue;
            }
        };

        let client_input = CreateClientParams {
            name: data.metadata.client.clone(),
            client_type: ClientType::Entity,
            email: None,
            phone: None,
            address: None,
            notes: None,
        };
        let client = match store
            .upsert_client_by_normalized_name(user_id, &client_input)
            .await
        {
            Ok(client) => client,
            Err(err) => {
                report.skipped_matters += 1;
                push_matter_reindex_warning(
                    &mut report,
                    format!(
                        "failed to upsert client for matter '{}': {}",
                        matter_id, err
                    ),
                );
                continue;
            }
        };

        let opened_at = match parse_optional_opened_at_ts(data.metadata.opened_at.as_deref()) {
            Ok(value) => value,
            Err(err) => {
                report.skipped_matters += 1;
                push_matter_reindex_warning(
                    &mut report,
                    format!(
                        "failed to parse opened_at for matter '{}': {}",
                        matter_id, err
                    ),
                );
                continue;
            }
        };

        let matter_input = UpsertMatterParams {
            matter_id: matter_id.clone(),
            client_id: client.id,
            status: data.status,
            stage: data.stage,
            practice_area: data.metadata.practice_area.clone(),
            jurisdiction: data.metadata.jurisdiction.clone(),
            opened_at,
            closed_at: None,
            assigned_to: data.metadata.team.clone(),
            custom_fields: serde_json::json!({}),
        };
        if let Err(err) = store.upsert_matter(user_id, &matter_input).await {
            report.skipped_matters += 1;
            push_matter_reindex_warning(
                &mut report,
                format!("failed to upsert matter '{}': {}", matter_id, err),
            );
            continue;
        }

        report.upserted_matters += 1;
    }

    Ok(report)
}

/// Rebuild the DB conflict graph from workspace matter metadata and
/// workspace-global `conflicts.json`.
pub async fn reindex_conflict_graph(
    workspace: &Workspace,
    store: &std::sync::Arc<dyn Database>,
    config: &LegalConfig,
) -> Result<ConflictGraphReindexReport, String> {
    let _reindex_guard = CONFLICT_REINDEX_LOCK.lock().await;

    let matter_root = config.matter_root.trim_matches('/');
    if matter_root.is_empty() {
        return Err("legal matter root is empty after normalization".to_string());
    }

    store
        .reset_conflict_graph()
        .await
        .map_err(|err| format!("failed to reset conflict graph: {err}"))?;

    let mut report = ConflictGraphReindexReport::default();
    let matter_entries = workspace
        .list(matter_root)
        .await
        .map_err(|err| format!("failed to list matter root '{matter_root}': {err}"))?;

    for entry in matter_entries
        .into_iter()
        .filter(|entry| entry.is_directory)
    {
        report.scanned_matters += 1;

        let raw_id = entry.path.rsplit('/').next().unwrap_or_default();
        let matter_id = sanitize_matter_id(raw_id);
        if matter_id.is_empty() {
            report.skipped_matters += 1;
            push_reindex_warning(
                &mut report,
                format!(
                    "skipped matter directory '{}' (empty id after sanitization)",
                    entry.path
                ),
            );
            continue;
        }

        let metadata = match read_matter_metadata_for_root(workspace, matter_root, &matter_id).await
        {
            Ok(metadata) => metadata,
            Err(err) => {
                report.skipped_matters += 1;
                push_reindex_warning(
                    &mut report,
                    format!(
                        "skipped matter '{}' due to invalid metadata: {}",
                        matter_id, err
                    ),
                );
                continue;
            }
        };

        match store
            .seed_matter_parties(
                &matter_id,
                &metadata.client,
                &metadata.adversaries,
                metadata.opened_at.as_deref(),
            )
            .await
        {
            Ok(_) => {
                report.seeded_matters += 1;
            }
            Err(err) => {
                report.skipped_matters += 1;
                push_reindex_warning(
                    &mut report,
                    format!(
                        "failed to seed matter '{}' into conflict graph: {}",
                        matter_id, err
                    ),
                );
            }
        }
    }

    match workspace.read("conflicts.json").await {
        Ok(doc) => {
            if let Some(entries) = parse_conflict_entries(&doc.content) {
                for entry in entries {
                    match store
                        .seed_conflict_entry(
                            GLOBAL_CONFLICT_GRAPH_MATTER_ID,
                            &entry.canonical_name,
                            &entry.aliases,
                            None,
                        )
                        .await
                    {
                        Ok(_) => {
                            report.global_conflicts_seeded += 1;
                            report.global_aliases_seeded += entry.aliases.len();
                        }
                        Err(err) => {
                            push_reindex_warning(
                                &mut report,
                                format!(
                                    "failed to seed global conflict '{}': {}",
                                    entry.canonical_name, err
                                ),
                            );
                            continue;
                        }
                    }
                }
            } else {
                push_reindex_warning(
                    &mut report,
                    "conflicts.json exists but could not be parsed; skipped global conflicts import"
                        .to_string(),
                );
            }
        }
        Err(WorkspaceError::DocumentNotFound { .. }) => {}
        Err(err) => {
            push_reindex_warning(
                &mut report,
                format!("failed to read conflicts.json during reindex: {}", err),
            );
        }
    }

    invalidate_conflict_cache();
    Ok(report)
}

/// Invalidate the cached conflicts.json parse result.
pub fn invalidate_conflict_cache() {
    let next_generation = CONFLICT_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    if let Ok(mut db_cache) = DB_CONFLICT_CACHE.lock() {
        db_cache.entries.clear();
        db_cache.generation = next_generation;
    }
}

/// True when the path resolves to the workspace-global `conflicts.json`.
pub fn is_workspace_conflicts_path(path: &str) -> bool {
    normalize_workspace_path(path) == "conflicts.json"
}

fn normalize_workspace_path(path: &str) -> String {
    let replaced = path.trim().replace('\\', "/");
    let mut parts: Vec<&str> = Vec::new();
    for component in replaced.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        parts.push(component);
    }
    parts.join("/")
}

fn normalize_conflict_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_sep = true;

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push(' ');
            last_was_sep = true;
        }
    }

    out.trim().to_string()
}

fn alias_is_matchable(alias: &str) -> bool {
    let mut token_count = 0usize;
    let mut first_token_len = 0usize;
    for token in alias.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        token_count += 1;
        if token_count == 1 {
            first_token_len = token.len();
        }
    }

    if token_count == 0 {
        return false;
    }

    if token_count > 1 {
        return true;
    }

    first_token_len >= MIN_ALIAS_SINGLE_TOKEN_LEN || alias.chars().any(|c| c.is_ascii_digit())
}

fn parse_conflict_entries(raw: &str) -> Option<Vec<ConflictEntry>> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    let entries = value.as_array()?;

    let mut parsed = Vec::new();
    for entry in entries {
        let canonical_name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim();
        if canonical_name.is_empty() {
            continue;
        }

        let normalized_name = normalize_conflict_text(canonical_name);
        if normalized_name.is_empty() {
            continue;
        }

        let mut terms = vec![normalized_name.clone()];
        let mut alias_values = Vec::new();
        let mut seen = std::collections::HashSet::new();
        seen.insert(normalized_name);

        if let Some(alias_entries) = entry.get("aliases").and_then(|v| v.as_array()) {
            for alias in alias_entries.iter().filter_map(|v| v.as_str()) {
                let alias_trimmed = alias.trim();
                if alias_trimmed.is_empty() {
                    continue;
                }
                let normalized_alias = normalize_conflict_text(alias);
                if normalized_alias.is_empty()
                    || !alias_is_matchable(&normalized_alias)
                    || !seen.insert(normalized_alias.clone())
                {
                    continue;
                }
                terms.push(normalized_alias);
                alias_values.push(alias_trimmed.to_string());
            }
        }

        parsed.push(ConflictEntry {
            canonical_name: canonical_name.to_string(),
            terms,
            aliases: alias_values,
        });
    }

    Some(parsed)
}

fn contains_term_with_boundaries(haystack: &str, term: &str) -> bool {
    if term.is_empty() {
        return false;
    }

    let bytes = haystack.as_bytes();
    let mut offset = 0usize;
    while let Some(rel_pos) = haystack[offset..].find(term) {
        let start = offset + rel_pos;
        let end = start + term.len();
        let before_ok = start == 0 || !bytes[start - 1].is_ascii_alphanumeric();
        let after_ok = end == bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        offset = start + 1;
    }

    false
}

fn detect_conflict_in_entries(
    entries: &[ConflictEntry],
    message: &str,
    active_matter: Option<&str>,
) -> Option<String> {
    let normalized_message = normalize_conflict_text(message);
    let normalized_active_matter = active_matter
        .map(normalize_conflict_text)
        .unwrap_or_default();

    for entry in entries {
        for term in &entry.terms {
            if contains_term_with_boundaries(&normalized_message, term)
                || contains_term_with_boundaries(&normalized_active_matter, term)
            {
                return Some(entry.canonical_name.clone());
            }
        }
    }

    None
}

fn detect_conflict_in_adversaries(
    adversaries: &[String],
    message: &str,
    active_matter: Option<&str>,
) -> Option<String> {
    let normalized_message = normalize_conflict_text(message);
    let normalized_active_matter = active_matter
        .map(normalize_conflict_text)
        .unwrap_or_default();
    if normalized_message.is_empty() && normalized_active_matter.is_empty() {
        return None;
    }

    for adversary in adversaries {
        let canonical_name = adversary.trim();
        if canonical_name.is_empty() {
            continue;
        }

        let normalized = normalize_conflict_text(canonical_name);
        if normalized.is_empty() || !alias_is_matchable(&normalized) {
            continue;
        }

        if contains_term_with_boundaries(&normalized_message, &normalized)
            || contains_term_with_boundaries(&normalized_active_matter, &normalized)
        {
            return Some(canonical_name.to_string());
        }
    }

    None
}

fn cache_snapshot() -> Option<(Vec<ConflictEntry>, bool)> {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    let cache = CONFLICT_CACHE.lock().ok()?;
    if !cache.ready {
        return None;
    }

    let within_window = cache
        .refreshed_at
        .is_some_and(|t| t.elapsed() <= CONFLICT_CACHE_REFRESH_WINDOW);
    let stale = cache.generation != generation || !within_window;
    Some((cache.entries.clone(), stale))
}

fn store_conflict_cache(entries: Vec<ConflictEntry>) {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    if let Ok(mut cache) = CONFLICT_CACHE.lock() {
        cache.entries = entries;
        cache.generation = generation;
        cache.refreshed_at = Some(Instant::now());
        cache.ready = true;
    }
    #[cfg(test)]
    {
        CONFLICT_CACHE_REFRESH_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

fn mark_conflict_cache_refresh_failure() {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    if let Ok(mut cache) = CONFLICT_CACHE.lock()
        && cache.ready
    {
        // Keep the stale snapshot for a bounded fallback window so temporary
        // read/parse failures do not cause repeated filesystem churn.
        cache.generation = generation;
        cache.refreshed_at = Some(Instant::now());
    }
}

fn db_conflict_cache_key(message: &str, active_matter: Option<&str>) -> String {
    let message = normalize_conflict_text(message);
    let matter = active_matter
        .map(normalize_conflict_text)
        .unwrap_or_default();
    format!("{message}|{matter}")
}

fn db_conflict_cache_lookup(key: &str) -> Option<Option<String>> {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    let cache = DB_CONFLICT_CACHE.lock().ok()?;
    if cache.generation != generation {
        return None;
    }
    let entry = cache.entries.get(key)?;
    if entry.refreshed_at.elapsed() > CONFLICT_CACHE_REFRESH_WINDOW {
        return None;
    }
    Some(entry.conflict.clone())
}

fn db_conflict_cache_store(key: String, conflict: Option<String>) {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    if let Ok(mut cache) = DB_CONFLICT_CACHE.lock() {
        if cache.generation != generation {
            cache.entries.clear();
            cache.generation = generation;
        }
        cache.entries.insert(
            key,
            DbConflictCacheEntry {
                conflict,
                refreshed_at: Instant::now(),
            },
        );
        if cache.entries.len() > 512 {
            cache
                .entries
                .retain(|_, value| value.refreshed_at.elapsed() <= CONFLICT_CACHE_REFRESH_WINDOW);
        }
    }
}

async fn detect_conflict_from_db(
    store: Option<&std::sync::Arc<dyn Database>>,
    config: &LegalConfig,
    message: &str,
) -> Option<String> {
    if !config.enabled || !config.conflict_check_enabled {
        return None;
    }
    let store = store?;
    let active_matter = config.active_matter.as_deref();
    let key = db_conflict_cache_key(message, active_matter);
    if let Some(cached) = db_conflict_cache_lookup(&key) {
        return cached;
    }

    let conflict = match store
        .find_conflict_hits_for_text(message, active_matter, 25)
        .await
    {
        Ok(hits) => hits.first().map(|hit| hit.party.clone()),
        Err(err) => {
            tracing::warn!("DB-backed conflict check failed, falling back to file cache: {err}");
            None
        }
    };
    db_conflict_cache_store(key, conflict.clone());
    conflict
}

async fn detect_conflict_from_workspace_conflicts(
    workspace: &Workspace,
    config: &LegalConfig,
    message: &str,
) -> Option<String> {
    if !config.enabled || !config.conflict_check_enabled {
        return None;
    }

    let active_matter = config.active_matter.as_deref();

    if let Some((entries, stale)) = cache_snapshot()
        && !stale
    {
        if let Some(conflict) = detect_conflict_in_entries(&entries, message, active_matter) {
            return Some(conflict);
        }
        if let Some(active_matter) = active_matter
            && let Ok(metadata) =
                read_matter_metadata_for_root(workspace, &config.matter_root, active_matter).await
        {
            return detect_conflict_in_adversaries(
                &metadata.adversaries,
                message,
                Some(active_matter),
            );
        }
        return None;
    }

    if let Some(doc) = workspace.read("conflicts.json").await.ok()
        && let Some(parsed) = parse_conflict_entries(&doc.content)
    {
        store_conflict_cache(parsed.clone());
        let global = detect_conflict_in_entries(&parsed, message, active_matter);
        if global.is_some() {
            return global;
        }
    } else {
        mark_conflict_cache_refresh_failure();
        if let Some((entries, _)) = cache_snapshot()
            && let Some(conflict) = detect_conflict_in_entries(&entries, message, active_matter)
        {
            return Some(conflict);
        }
    }

    if let Some(active_matter) = active_matter
        && let Ok(metadata) =
            read_matter_metadata_for_root(workspace, &config.matter_root, active_matter).await
    {
        return detect_conflict_in_adversaries(&metadata.adversaries, message, Some(active_matter));
    }

    None
}

/// Check for conflict hits using DB-backed matching first, then fallback to
/// workspace `conflicts.json` + active-matter adversaries.
pub async fn detect_conflict_with_store(
    store: Option<&std::sync::Arc<dyn Database>>,
    workspace: &Workspace,
    config: &LegalConfig,
    message: &str,
) -> Option<String> {
    if !config.enabled || !config.conflict_check_enabled {
        return None;
    }
    let db_available = store.is_some();
    if let Some(conflict) = detect_conflict_from_db(store, config, message).await {
        return Some(conflict);
    }
    if db_available && !config.conflict_file_fallback_enabled {
        return None;
    }
    detect_conflict_from_workspace_conflicts(workspace, config, message).await
}

/// Backward-compatible detector for call sites that do not have DB access.
pub async fn detect_conflict(
    workspace: &Workspace,
    config: &LegalConfig,
    message: &str,
) -> Option<String> {
    detect_conflict_with_store(None, workspace, config, message).await
}

#[cfg(test)]
pub(crate) fn reset_conflict_cache_for_tests() {
    CONFLICT_CACHE_GENERATION.store(1, Ordering::Relaxed);
    CONFLICT_CACHE_REFRESH_COUNT.store(0, Ordering::Relaxed);
    if let Ok(mut cache) = CONFLICT_CACHE.lock() {
        *cache = ConflictCacheState::default();
    }
    if let Ok(mut db_cache) = DB_CONFLICT_CACHE.lock() {
        *db_cache = DbConflictCacheState::default();
    }
}

#[cfg(test)]
pub(crate) fn conflict_cache_refresh_count_for_tests() -> u64 {
    CONFLICT_CACHE_REFRESH_COUNT.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::{
        MatterMetadata, alias_is_matchable, contains_term_with_boundaries,
        is_workspace_conflicts_path, matter_metadata_path_for_root, normalize_conflict_text,
        parse_conflict_entries, sanitize_prompt_field, sanitize_prompt_list,
    };

    #[test]
    fn matter_metadata_requires_core_fields() {
        let missing = MatterMetadata {
            matter_id: "".to_string(),
            client: "".to_string(),
            team: vec![],
            confidentiality: "".to_string(),
            adversaries: vec![],
            retention: "".to_string(),
            jurisdiction: None,
            practice_area: None,
            opened_at: None,
        };
        assert!(missing.validate_required_fields().is_err());

        let ok = MatterMetadata {
            matter_id: "acme-v-foo".to_string(),
            client: "Acme".to_string(),
            team: vec!["Lead Counsel".to_string()],
            confidentiality: "attorney-client-privileged".to_string(),
            adversaries: vec!["Foo Corp".to_string()],
            retention: "follow-firm-policy".to_string(),
            jurisdiction: None,
            practice_area: None,
            opened_at: None,
        };
        assert!(ok.validate_required_fields().is_ok());
    }

    #[test]
    fn matter_metadata_parses_optional_context_fields() {
        let parsed: MatterMetadata = serde_yml::from_str(
            r#"
matter_id: acme-v-foo
client: Acme Corp
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
jurisdiction: SDNY / Delaware
practice_area: commercial litigation
opened_at: 2024-03-15
"#,
        )
        .expect("yaml should parse");

        assert_eq!(parsed.jurisdiction.as_deref(), Some("SDNY / Delaware"));
        assert_eq!(
            parsed.practice_area.as_deref(),
            Some("commercial litigation")
        );
        assert_eq!(parsed.opened_at.as_deref(), Some("2024-03-15"));
    }

    #[test]
    fn matter_metadata_path_for_root_normalizes_id() {
        assert_eq!(
            matter_metadata_path_for_root("matters", "Acme v. Foo"),
            "matters/acme-v--foo/matter.yaml"
        );
    }

    #[test]
    fn conflict_path_normalization_handles_variants() {
        assert!(is_workspace_conflicts_path("conflicts.json"));
        assert!(is_workspace_conflicts_path("./conflicts.json"));
        assert!(is_workspace_conflicts_path("///./conflicts.json///"));
        assert!(!is_workspace_conflicts_path("matters/demo/conflicts.json"));
    }

    #[test]
    fn conflict_match_normalization_and_boundaries_work() {
        let haystack = normalize_conflict_text("Counsel discussed Example-Co, Inc. today.");
        assert!(contains_term_with_boundaries(
            &haystack,
            &normalize_conflict_text("example co inc")
        ));
        assert!(!contains_term_with_boundaries(
            &haystack,
            &normalize_conflict_text("ample")
        ));

        let corporation_haystack = normalize_conflict_text("I became active in the corporation.");
        assert!(!contains_term_with_boundaries(
            &corporation_haystack,
            &normalize_conflict_text("corp")
        ));
        let corp_haystack = normalize_conflict_text("This matter references Acme Corp directly.");
        assert!(contains_term_with_boundaries(
            &corp_haystack,
            &normalize_conflict_text("corp")
        ));
    }

    #[test]
    fn short_alias_guardrails_skip_noisy_single_token_aliases() {
        assert!(!alias_is_matchable("ab"));
        assert!(!alias_is_matchable("xyz"));
        assert!(alias_is_matchable("acme"));
        assert!(alias_is_matchable("acme co"));
    }

    #[test]
    fn parse_conflicts_applies_alias_guardrails() {
        let parsed = parse_conflict_entries(
            r#"[{"name":"Example Adverse Party","aliases":["EA","Example Co","x1"]}]"#,
        )
        .expect("valid conflicts json");

        assert_eq!(parsed.len(), 1);
        let terms = &parsed[0].terms;
        assert!(terms.iter().any(|t| t == "example adverse party"));
        assert!(terms.iter().any(|t| t == "example co"));
        assert!(!terms.iter().any(|t| t == "ea"));
        assert!(parsed[0].aliases.iter().any(|alias| alias == "Example Co"));
        assert!(!parsed[0].aliases.iter().any(|alias| alias == "EA"));
    }

    #[test]
    fn prompt_field_sanitization_normalizes_whitespace_and_controls() {
        let raw = "  Acme\tCorp\n\x07Litigation   Team  ";
        let cleaned = sanitize_prompt_field(raw, 120);
        assert_eq!(cleaned, "Acme Corp Litigation Team");
    }

    #[test]
    fn prompt_list_sanitization_applies_limits_and_drops_empty() {
        let values = vec![
            " Lead Counsel ".to_string(),
            "\n\n".to_string(),
            "Associate\tOne".to_string(),
            "Paralegal".to_string(),
            "Investigator".to_string(),
            "Expert".to_string(),
            "Analyst".to_string(),
            "Clerk".to_string(),
            "Runner".to_string(),
        ];
        let cleaned = sanitize_prompt_list(&values);
        assert_eq!(cleaned.len(), 8);
        assert_eq!(cleaned[0], "Lead Counsel");
        assert!(cleaned.iter().all(|item| !item.is_empty()));
    }
}

#[cfg(test)]
mod cache_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::{
        CONFLICT_REINDEX_LOCK, GLOBAL_CONFLICT_GRAPH_MATTER_ID,
        conflict_cache_refresh_count_for_tests, detect_conflict, invalidate_conflict_cache,
        load_active_matter_prompt_context, reindex_conflict_graph, reset_conflict_cache_for_tests,
    };
    use crate::config::LegalConfig;
    use crate::settings::Settings;
    use crate::workspace::Workspace;

    // All cache tests share a process-wide global; run them serially so they
    // don't stomp each other's `CONFLICT_CACHE_REFRESH_COUNT` state.
    static CACHE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_uses_cache_until_invalidated() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));

        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Example Co","aliases":["Example Company","ExCo"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.active_matter = None;
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let first = detect_conflict(workspace.as_ref(), &legal, "Representing Example Co").await;
        assert_eq!(first.as_deref(), Some("Example Co"));
        assert_eq!(
            conflict_cache_refresh_count_for_tests(),
            1,
            "first lookup should parse conflicts.json once"
        );

        let second =
            detect_conflict(workspace.as_ref(), &legal, "Example Company is mentioned").await;
        assert_eq!(second.as_deref(), Some("Example Co"));
        assert_eq!(
            conflict_cache_refresh_count_for_tests(),
            1,
            "second lookup should reuse cached conflicts parse"
        );

        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Beta Corp","aliases":["Beta"]}]"#,
            )
            .await
            .expect("update conflicts");
        invalidate_conflict_cache();

        let third = detect_conflict(workspace.as_ref(), &legal, "New issue with Beta Corp").await;
        assert_eq!(third.as_deref(), Some("Beta Corp"));
        assert_eq!(
            conflict_cache_refresh_count_for_tests(),
            2,
            "cache invalidation should force a refresh on next lookup"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_with_store_respects_disabled_policy() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Example Co","aliases":["Example Company"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = false;
        legal.active_matter = None;

        let hit = super::detect_conflict_with_store(
            Some(&db),
            workspace.as_ref(),
            &legal,
            "Representing Example Co",
        )
        .await;
        assert_eq!(hit, None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_with_store_can_disable_file_fallback_when_db_is_available() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Fallback Only Party","aliases":["Fallback Co"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;
        legal.conflict_file_fallback_enabled = false;

        let hit = super::detect_conflict_with_store(
            Some(&db),
            workspace.as_ref(),
            &legal,
            "Discussing Fallback Only Party strategy",
        )
        .await;
        assert_eq!(
            hit, None,
            "DB-authoritative mode should not use conflicts.json fallback"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_without_store_still_uses_file_fallback() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Fallback Party","aliases":["Fallback Co"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;
        legal.conflict_file_fallback_enabled = false;

        let hit = super::detect_conflict_with_store(
            None,
            workspace.as_ref(),
            &legal,
            "Discussing Fallback Party strategy",
        )
        .await;
        assert_eq!(hit.as_deref(), Some("Fallback Party"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_matches_active_matter_adversary_without_conflicts_json_hit() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
"#,
            )
            .await
            .expect("seed matter metadata");
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Different Party","aliases":["Different"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.active_matter = Some("demo".to_string());
        legal.conflict_check_enabled = true;

        let hit = detect_conflict(
            workspace.as_ref(),
            &legal,
            "Did Foo Industries contact us about pricing?",
        )
        .await;
        assert_eq!(hit.as_deref(), Some("Foo Industries"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_uses_warm_cache_for_no_match_without_refreshing_disk_parse() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Alpha Holdings","aliases":["Alpha"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let first = detect_conflict(workspace.as_ref(), &legal, "No listed parties here").await;
        assert_eq!(first, None);
        assert_eq!(
            conflict_cache_refresh_count_for_tests(),
            1,
            "first call should parse conflicts.json once"
        );

        let second = detect_conflict(workspace.as_ref(), &legal, "Still no listed parties").await;
        assert_eq!(second, None);
        assert_eq!(
            conflict_cache_refresh_count_for_tests(),
            1,
            "warm-cache no-match path should not re-read conflicts.json"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_ignores_short_single_token_adversary_false_positive() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Corp
retention: follow-firm-policy
"#,
            )
            .await
            .expect("seed matter metadata");
        workspace
            .write("conflicts.json", "[]")
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.active_matter = Some("demo".to_string());
        legal.conflict_check_enabled = true;

        let hit = detect_conflict(
            workspace.as_ref(),
            &legal,
            "We became active in the corporation",
        )
        .await;
        assert_eq!(hit, None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_matches_active_matter_identifier_against_adversaries() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/smith-v-acme-corp/matter.yaml",
                r#"
matter_id: smith-v-acme-corp
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Acme Corp
retention: follow-firm-policy
"#,
            )
            .await
            .expect("seed matter metadata");
        workspace
            .write("conflicts.json", "[]")
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.active_matter = Some("smith-v-acme-corp".to_string());
        legal.conflict_check_enabled = true;

        let hit =
            detect_conflict(workspace.as_ref(), &legal, "General project planning note").await;
        assert_eq!(hit.as_deref(), Some("Acme Corp"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn reindex_conflict_graph_backfills_workspace_matters_and_global_conflicts() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
opened_at: 2026-02-28
"#,
            )
            .await
            .expect("seed matter metadata");
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Example Adverse Party","aliases":["Example Co"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let report = reindex_conflict_graph(workspace.as_ref(), &db, &legal)
            .await
            .expect("reindex should succeed");
        assert_eq!(report.scanned_matters, 1);
        assert_eq!(report.seeded_matters, 1);
        assert_eq!(report.global_conflicts_seeded, 1);
        assert_eq!(report.global_aliases_seeded, 1);

        let demo_hit = db
            .find_conflict_hits_for_names(&["Demo Client".to_string()], 20)
            .await
            .expect("query hits");
        assert!(demo_hit.iter().any(|hit| hit.matter_id == "demo"));

        let global_hit = db
            .find_conflict_hits_for_names(&["Example Co".to_string()], 20)
            .await
            .expect("query global alias hit");
        assert!(
            global_hit
                .iter()
                .any(|hit| hit.matter_id == GLOBAL_CONFLICT_GRAPH_MATTER_ID),
            "global conflicts should be queryable from DB graph"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn reindex_conflict_graph_waits_for_global_reindex_lock() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
"#,
            )
            .await
            .expect("seed matter metadata");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let reindex_lock_guard = CONFLICT_REINDEX_LOCK.lock().await;
        let workspace_for_task = Arc::clone(&workspace);
        let db_for_task = Arc::clone(&db);
        let legal_for_task = legal.clone();

        let mut reindex_task = tokio::spawn(async move {
            reindex_conflict_graph(workspace_for_task.as_ref(), &db_for_task, &legal_for_task).await
        });

        let blocked = tokio::time::timeout(Duration::from_millis(100), &mut reindex_task).await;
        assert!(blocked.is_err(), "reindex should wait while lock is held");

        drop(reindex_lock_guard);
        let report = reindex_task
            .await
            .expect("reindex task join should succeed")
            .expect("reindex should succeed");
        assert_eq!(report.seeded_matters, 1);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn detect_conflict_read_path_not_blocked_by_reindex_lock() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Example Co","aliases":["Example Company","ExCo"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let _reindex_lock_guard = CONFLICT_REINDEX_LOCK.lock().await;
        let hit = tokio::time::timeout(
            Duration::from_millis(250),
            detect_conflict(workspace.as_ref(), &legal, "Representing Example Co"),
        )
        .await
        .expect("conflict read should not block on reindex lock");
        assert_eq!(hit.as_deref(), Some("Example Co"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn load_active_matter_prompt_context_includes_optional_fields_when_present() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
jurisdiction: SDNY / Delaware
practice_area: commercial litigation
opened_at: 2024-03-15
"#,
            )
            .await
            .expect("seed matter metadata");

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.active_matter = Some("demo".to_string());

        let ctx = load_active_matter_prompt_context(workspace.as_ref(), &legal)
            .await
            .expect("context load should succeed")
            .expect("active matter context should be present");
        assert_eq!(ctx.jurisdiction.as_deref(), Some("SDNY / Delaware"));
        assert_eq!(ctx.practice_area.as_deref(), Some("commercial litigation"));
        assert_eq!(ctx.opened_at.as_deref(), Some("2024-03-15"));
    }
}
