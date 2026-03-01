use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::config::LegalConfig;
use crate::error::WorkspaceError;
use crate::legal::policy::sanitize_matter_id;
use crate::workspace::Workspace;

const CONFLICT_CACHE_REFRESH_WINDOW: Duration = Duration::from_secs(30);
const MIN_ALIAS_SINGLE_TOKEN_LEN: usize = 4;
const MATTER_PROMPT_LIST_MAX_ITEMS: usize = 8;
const MATTER_PROMPT_FIELD_MAX_CHARS: usize = 160;
const MATTER_PROMPT_LIST_ITEM_MAX_CHARS: usize = 96;

#[derive(Debug, Clone)]
struct ConflictEntry {
    canonical_name: String,
    terms: Vec<String>,
}

#[derive(Debug, Default)]
struct ConflictCacheState {
    entries: Vec<ConflictEntry>,
    generation: u64,
    refreshed_at: Option<Instant>,
    failure_started_at: Option<Instant>,
    ready: bool,
}

static CONFLICT_CACHE: LazyLock<Mutex<ConflictCacheState>> =
    LazyLock::new(|| Mutex::new(ConflictCacheState::default()));
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMatterPromptContext {
    pub matter_id: String,
    pub client: String,
    pub confidentiality: String,
    pub retention: String,
    pub team: Vec<String>,
    pub adversaries: Vec<String>,
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

/// Invalidate the cached conflicts.json parse result.
pub fn invalidate_conflict_cache() {
    CONFLICT_CACHE_GENERATION.fetch_add(1, Ordering::Relaxed);
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
        let mut seen = std::collections::HashSet::new();
        seen.insert(normalized_name);

        if let Some(aliases) = entry.get("aliases").and_then(|v| v.as_array()) {
            for alias in aliases.iter().filter_map(|v| v.as_str()) {
                let normalized_alias = normalize_conflict_text(alias);
                if normalized_alias.is_empty()
                    || !alias_is_matchable(&normalized_alias)
                    || !seen.insert(normalized_alias.clone())
                {
                    continue;
                }
                terms.push(normalized_alias);
            }
        }

        parsed.push(ConflictEntry {
            canonical_name: canonical_name.to_string(),
            terms,
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

fn cache_snapshot() -> Option<(Vec<ConflictEntry>, bool)> {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    let cache = CONFLICT_CACHE.lock().ok()?;
    if !cache.ready {
        return None;
    }

    let within_window = if let Some(failure_started_at) = cache.failure_started_at {
        failure_started_at.elapsed() <= CONFLICT_CACHE_REFRESH_WINDOW
    } else {
        cache
            .refreshed_at
            .is_some_and(|t| t.elapsed() <= CONFLICT_CACHE_REFRESH_WINDOW)
    };
    let stale = cache.generation != generation || !within_window;
    Some((cache.entries.clone(), stale))
}

fn store_conflict_cache(entries: Vec<ConflictEntry>) {
    let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
    if let Ok(mut cache) = CONFLICT_CACHE.lock() {
        cache.entries = entries;
        cache.generation = generation;
        cache.refreshed_at = Some(Instant::now());
        cache.failure_started_at = None;
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
        // Keep a stale snapshot for one bounded fallback window when reads/parses fail.
        // Do not extend the window on repeated failures.
        cache.generation = generation;
        if cache.failure_started_at.is_none() {
            cache.failure_started_at = Some(Instant::now());
        }
    }
}

/// Check conflicts.json for conflict hits in message or active matter.
pub async fn detect_conflict(
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
        return detect_conflict_in_entries(&entries, message, active_matter);
    }

    if let Some(doc) = workspace.read("conflicts.json").await.ok()
        && let Some(parsed) = parse_conflict_entries(&doc.content)
    {
        store_conflict_cache(parsed.clone());
        return detect_conflict_in_entries(&parsed, message, active_matter);
    }

    mark_conflict_cache_refresh_failure();
    if let Some((entries, stale)) = cache_snapshot()
        && !stale
    {
        return detect_conflict_in_entries(&entries, message, active_matter);
    }

    None
}

#[cfg(test)]
pub(crate) fn reset_conflict_cache_for_tests() {
    CONFLICT_CACHE_GENERATION.store(1, Ordering::Relaxed);
    CONFLICT_CACHE_REFRESH_COUNT.store(0, Ordering::Relaxed);
    if let Ok(mut cache) = CONFLICT_CACHE.lock() {
        *cache = ConflictCacheState::default();
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
        };
        assert!(missing.validate_required_fields().is_err());

        let ok = MatterMetadata {
            matter_id: "acme-v-foo".to_string(),
            client: "Acme".to_string(),
            team: vec!["Lead Counsel".to_string()],
            confidentiality: "attorney-client-privileged".to_string(),
            adversaries: vec!["Foo Corp".to_string()],
            retention: "follow-firm-policy".to_string(),
        };
        assert!(ok.validate_required_fields().is_ok());
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

#[cfg(all(test, feature = "libsql"))]
mod cache_tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    use crate::config::LegalConfig;
    use crate::settings::Settings;
    use crate::workspace::Workspace;

    use super::{
        CONFLICT_CACHE, CONFLICT_CACHE_GENERATION, CONFLICT_CACHE_REFRESH_WINDOW, ConflictEntry,
        conflict_cache_refresh_count_for_tests, detect_conflict, invalidate_conflict_cache,
        mark_conflict_cache_refresh_failure, normalize_conflict_text,
        reset_conflict_cache_for_tests,
    };

    static CACHE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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

    #[test]
    fn failure_fallback_window_is_not_extended_by_repeated_failures() {
        reset_conflict_cache_for_tests();
        let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
        {
            let mut cache = CONFLICT_CACHE.lock().expect("cache lock");
            cache.ready = true;
            cache.generation = generation;
            cache.refreshed_at = Some(Instant::now());
            cache.failure_started_at = None;
        }

        mark_conflict_cache_refresh_failure();
        let first = {
            let cache = CONFLICT_CACHE.lock().expect("cache lock");
            cache
                .failure_started_at
                .expect("failure window should be set")
        };
        mark_conflict_cache_refresh_failure();
        let second = {
            let cache = CONFLICT_CACHE.lock().expect("cache lock");
            cache
                .failure_started_at
                .expect("failure window should remain set")
        };

        assert_eq!(
            first, second,
            "repeated failures should not extend stale fallback window"
        );
    }

    #[tokio::test]
    async fn detect_conflict_does_not_use_expired_failure_fallback_cache() {
        let _guard = CACHE_TEST_LOCK.lock().await;
        reset_conflict_cache_for_tests();

        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));

        let generation = CONFLICT_CACHE_GENERATION.load(Ordering::Relaxed);
        {
            let mut cache = CONFLICT_CACHE.lock().expect("cache lock");
            cache.entries = vec![ConflictEntry {
                canonical_name: "Alpha Holdings".to_string(),
                terms: vec![normalize_conflict_text("Alpha Holdings")],
            }];
            cache.generation = generation;
            cache.refreshed_at = Some(Instant::now());
            cache.failure_started_at =
                Some(Instant::now() - CONFLICT_CACHE_REFRESH_WINDOW - Duration::from_secs(1));
            cache.ready = true;
        }

        let mut legal =
            LegalConfig::resolve(&Settings::default()).expect("default legal config resolves");
        legal.enabled = true;
        legal.conflict_check_enabled = true;
        legal.active_matter = None;

        let hit = detect_conflict(
            workspace.as_ref(),
            &legal,
            "Please check conflict for Alpha Holdings",
        )
        .await;
        assert_eq!(
            hit, None,
            "expired fallback window must not keep serving stale conflicts"
        );
    }
}
