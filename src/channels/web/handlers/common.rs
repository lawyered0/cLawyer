//! Shared web handler helpers and policy utilities.

use std::path::{Component as FsComponent, Path as FsPath};
use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    AuditSeverity, ClientType, CreateClientParams, CreateDocumentVersionParams,
    CreateMatterDeadlineParams, ExpenseCategory, InvoiceLineItemRecord, InvoiceRecord,
    MatterDeadlineType, MatterDocumentCategory, MatterStatus, MatterTaskStatus,
    TrustLedgerEntryRecord, UpsertDocumentTemplateParams, UpsertMatterDocumentParams,
    UpsertMatterParams,
};
use crate::workspace::{Workspace, paths};

pub(crate) const MATTER_ROOT: &str = "matters";
/// Settings key used to persist the active matter ID.
pub(crate) const MATTER_ACTIVE_SETTING: &str = "legal.active_matter";
/// Maximum number of party names accepted by intake conflict endpoints.
pub(crate) const MAX_INTAKE_CONFLICT_PARTIES: usize = 64;
/// Maximum length for a single intake party name.
const MAX_INTAKE_CONFLICT_PARTY_CHARS: usize = 160;
/// Maximum reminder offsets accepted for a single deadline.
const MAX_DEADLINE_REMINDERS: usize = 16;
/// Maximum allowed reminder offset in days.
const MAX_DEADLINE_REMINDER_DAYS: i32 = 3650;
/// Maximum allowed body text length for `/api/matters/conflicts/check`.
pub(crate) const MAX_CONFLICT_CHECK_TEXT_LEN: usize = 32 * 1024;
/// Default invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_DEFAULT_LIMIT: usize = 25;
/// Maximum invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_MAX_LIMIT: usize = 100;

/// Identity files that must not be overwritten through web memory-write APIs.
const PROTECTED_IDENTITY_FILES: &[&str] =
    &[paths::IDENTITY, paths::SOUL, paths::AGENTS, paths::USER];

pub(crate) fn legal_config_for_gateway(state: &GatewayState) -> crate::config::LegalConfig {
    state.legal_config.clone().unwrap_or_else(|| {
        crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve")
    })
}

pub(crate) fn matter_root_for_gateway(state: &GatewayState) -> String {
    let configured = legal_config_for_gateway(state).matter_root;
    let normalized = configured.trim_matches('/');
    if normalized.is_empty() {
        MATTER_ROOT.to_string()
    } else {
        normalized.to_string()
    }
}

fn matter_prefix_for_gateway(state: &GatewayState, matter_id: &str) -> String {
    format!("{}/{matter_id}", matter_root_for_gateway(state))
}

pub(crate) fn matter_metadata_path_for_gateway(state: &GatewayState, matter_id: &str) -> String {
    format!(
        "{}/matter.yaml",
        matter_prefix_for_gateway(state, matter_id)
    )
}

/// Normalize user-supplied memory paths for policy checks.
///
/// This mirrors workspace normalization semantics that strip leading/trailing
/// slashes, collapse duplicate separators, and ignore `.` segments.
/// `..` segments are preserved and rejected separately by traversal guards.
fn normalize_policy_path(path: &str) -> String {
    let mut parts = Vec::new();
    for component in FsPath::new(path.trim()).components() {
        match component {
            FsComponent::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            FsComponent::ParentDir => parts.push("..".to_string()),
            FsComponent::CurDir | FsComponent::RootDir | FsComponent::Prefix(_) => {}
        }
    }
    parts.join("/")
}

fn is_protected_identity_path(path: &str) -> bool {
    let normalized = normalize_policy_path(path);
    PROTECTED_IDENTITY_FILES
        .iter()
        .any(|protected| normalized.eq_ignore_ascii_case(protected))
}

pub(crate) async fn resolve_memory_write_path_for_gateway(
    state: &GatewayState,
    requested_path: &str,
) -> Result<String, (StatusCode, String)> {
    let normalized = normalize_policy_path(requested_path);
    if normalized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Path is empty after normalization".to_string(),
        ));
    }

    if is_protected_identity_path(&normalized) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "Path '{}' is protected from tool/web writes",
                requested_path
            ),
        ));
    }

    let legal = legal_config_for_gateway(state);
    let resolved_path = if legal.enabled && legal.require_matter_context {
        let active_matter = if let Some(store) = state.store.as_ref() {
            match store
                .get_setting(&state.user_id, MATTER_ACTIVE_SETTING)
                .await
            {
                Ok(value) => value
                    .and_then(|raw| raw.as_str().map(str::to_owned))
                    .and_then(|raw| crate::legal::policy::sanitize_optional_matter_id(&raw)),
                Err(err) => {
                    tracing::warn!(
                        "Failed to load active matter setting for memory write policy: {}",
                        err
                    );
                    None
                }
            }
        } else {
            None
        };
        let matter_id = active_matter
            .or_else(|| legal.active_matter.clone())
            .ok_or((
                StatusCode::FORBIDDEN,
                "No active matter selected. Set an active matter before writing files.".to_string(),
            ))?;
        let matter_root = matter_root_for_gateway(state);
        let matter_prefix = format!("{matter_root}/{matter_id}");
        let matter_root_prefix = format!("{matter_root}/");

        if normalized == matter_prefix || normalized.starts_with(&format!("{matter_prefix}/")) {
            normalized
        } else if normalized == matter_root || normalized.starts_with(&matter_root_prefix) {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "Path '{}' is outside active matter scope '{}'",
                    requested_path, matter_prefix
                ),
            ));
        } else {
            format!("{matter_prefix}/{normalized}")
        }
    } else {
        normalized
    };

    if FsPath::new(&resolved_path)
        .components()
        .any(|component| component == FsComponent::ParentDir)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Path '{}' contains directory traversal sequences",
                requested_path
            ),
        ));
    }

    if is_protected_identity_path(&resolved_path) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "Path '{}' resolves to a protected identity file",
                requested_path
            ),
        ));
    }

    Ok(resolved_path)
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegalAuditQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
    pub(crate) event_type: Option<String>,
    pub(crate) matter_id: Option<String>,
    pub(crate) severity: Option<String>,
    pub(crate) since: Option<String>,
    pub(crate) until: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
}

pub(crate) fn parse_utc_query_ts(
    field_name: &str,
    raw: Option<&str>,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid RFC3339 timestamp", field_name),
        )
    })?;
    Ok(Some(parsed.with_timezone(&Utc)))
}

pub(crate) fn parse_audit_severity_query(
    raw: Option<&str>,
) -> Result<Option<AuditSeverity>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "info" => Ok(Some(AuditSeverity::Info)),
        "warn" => Ok(Some(AuditSeverity::Warn)),
        "critical" => Ok(Some(AuditSeverity::Critical)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'severity' must be one of: info, warn, critical".to_string(),
        )),
    }
}

pub(crate) async fn record_legal_audit_event(
    state: &GatewayState,
    event_type: &str,
    actor: &str,
    matter_id: Option<&str>,
    severity: AuditSeverity,
    details: serde_json::Value,
) {
    if let Some(store) = state.store.as_ref() {
        crate::legal::audit::record_with_db(
            event_type,
            actor,
            matter_id,
            severity,
            details,
            store.as_ref(),
            &state.user_id,
        )
        .await;
    } else {
        crate::legal::audit::record(event_type, details);
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MatterDocumentsQuery {
    pub(crate) include_templates: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MatterInvoicesQuery {
    pub(crate) limit: Option<usize>,
}
#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
pub(crate) struct ClientsQuery {
    pub(crate) q: Option<String>,
}

pub(crate) fn sanitize_matter_id_for_route(raw: &str) -> Result<String, (StatusCode, String)> {
    let sanitized = crate::legal::policy::sanitize_matter_id(raw);
    if sanitized.is_empty() {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    Ok(sanitized)
}

pub(crate) async fn ensure_existing_matter_for_route(
    workspace: &Workspace,
    matter_root: &str,
    raw_matter_id: &str,
) -> Result<String, (StatusCode, String)> {
    let matter_id = sanitize_matter_id_for_route(raw_matter_id)?;
    match crate::legal::matter::read_matter_metadata_for_root(workspace, matter_root, &matter_id)
        .await
    {
        Ok(_) => Ok(matter_id),
        Err(crate::legal::matter::MatterMetadataValidationError::Missing { path }) => Err((
            StatusCode::NOT_FOUND,
            format!("Matter '{}' not found (missing '{}')", matter_id, path),
        )),
        Err(crate::legal::matter::MatterMetadataValidationError::Invalid { .. }) => Err((
            StatusCode::NOT_FOUND,
            format!("Matter '{}' metadata is invalid", matter_id),
        )),
        Err(err @ crate::legal::matter::MatterMetadataValidationError::Storage { .. }) => {
            Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
        }
    }
}

pub(crate) fn parse_template_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must not be empty".to_string(),
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must be a basename under templates/".to_string(),
        ));
    }
    let path = FsPath::new(trimmed);
    let basename = path.file_name().and_then(|value| value.to_str()).ok_or((
        StatusCode::BAD_REQUEST,
        "'template_name' must be valid UTF-8".to_string(),
    ))?;
    if basename != trimmed {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must be a basename under templates/".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) async fn choose_template_apply_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    template_name: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    let template_path = FsPath::new(template_name);
    let stem = template_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "'template_name' must include a valid file stem".to_string(),
        ))?;
    let ext = template_path.extension().and_then(|value| value.to_str());

    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let file_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}-{timestamp}{suffix}.{ext}"),
            _ => format!("{stem}-{timestamp}{suffix}"),
        };
        let candidate = format!("{matter_prefix}/drafts/{file_name}");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to pick a unique destination for applied template".to_string(),
    ))
}

pub(crate) async fn choose_generated_document_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    template_name: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    let parsed = FsPath::new(template_name);
    let stem = parsed
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("generated-document");
    let ext = parsed
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("md");

    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let candidate = format!("{matter_prefix}/drafts/{stem}-{timestamp}{suffix}.{ext}");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to pick a unique destination for generated document".to_string(),
    ))
}

pub(crate) async fn list_matter_documents_recursive(
    workspace: &Workspace,
    matter_prefix: &str,
    include_templates: bool,
) -> Result<Vec<MatterDocumentInfo>, (StatusCode, String)> {
    let mut pending = vec![matter_prefix.to_string()];
    let mut documents = Vec::new();
    let templates_prefix = format!("{matter_prefix}/templates");

    while let Some(path) = pending.pop() {
        let entries = workspace
            .list(&path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

        for entry in entries {
            if !include_templates
                && (entry.path == templates_prefix
                    || entry.path.starts_with(&format!("{templates_prefix}/")))
            {
                continue;
            }

            let name = entry.path.rsplit('/').next().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }

            documents.push(MatterDocumentInfo {
                id: None,
                memory_document_id: None,
                name,
                display_name: None,
                path: entry.path.clone(),
                is_dir: entry.is_directory,
                category: None,
                updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
            });

            if entry.is_directory {
                pending.push(entry.path);
            }
        }
    }

    documents.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(documents)
}

pub(crate) fn checklist_completion_from_markdown(markdown: &str) -> (usize, usize) {
    let mut completed = 0usize;
    let mut total = 0usize;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = if let Some(rest) = trimmed.strip_prefix("- [") {
            rest
        } else {
            continue;
        };
        let mut chars = marker.chars();
        let state = chars.next().unwrap_or(' ');
        let bracket = chars.next().unwrap_or(' ');
        if bracket != ']' {
            continue;
        }
        total += 1;
        if matches!(state, 'x' | 'X' | '✓') {
            completed += 1;
        }
    }

    (completed, total)
}

fn parse_iso_date_token(input: &str) -> Option<(NaiveDate, usize, usize)> {
    let bytes = input.as_bytes();
    if bytes.len() < 10 {
        return None;
    }

    for start in 0..=bytes.len() - 10 {
        let token = &bytes[start..start + 10];
        let is_iso = token[0].is_ascii_digit()
            && token[1].is_ascii_digit()
            && token[2].is_ascii_digit()
            && token[3].is_ascii_digit()
            && token[4] == b'-'
            && token[5].is_ascii_digit()
            && token[6].is_ascii_digit()
            && token[7] == b'-'
            && token[8].is_ascii_digit()
            && token[9].is_ascii_digit();
        if !is_iso {
            continue;
        }

        let Ok(token_str) = std::str::from_utf8(token) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(token_str, "%Y-%m-%d") else {
            continue;
        };
        return Some((date, start, start + 10));
    }

    None
}

fn parse_deadlines_from_calendar(markdown: &str, today: NaiveDate) -> Vec<MatterDeadlineInfo> {
    let mut deadlines: Vec<(NaiveDate, MatterDeadlineInfo)> = Vec::new();

    for raw_line in markdown.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("|---") {
            continue;
        }

        // Parse table rows first: | Date | Deadline / Event | Owner | Status | Source |
        if line.starts_with('|') {
            let normalized = line.trim_matches('|').trim();
            if normalized.is_empty() {
                continue;
            }
            let columns: Vec<&str> = line.split('|').map(str::trim).collect();
            // split('|') on a pipe-delimited row includes leading/trailing empty tokens.
            // We trim those by slicing, but keep interior empties to preserve column positions.
            let columns = if columns.len() >= 2 {
                &columns[1..columns.len() - 1]
            } else {
                &columns[..]
            };
            if columns.len() < 2
                || columns[0].eq_ignore_ascii_case("date")
                || columns[1].eq_ignore_ascii_case("deadline / event")
            {
                continue;
            }
            if let Some((date, _, _)) = parse_iso_date_token(columns[0]) {
                let title = columns.get(1).copied().unwrap_or("").trim().to_string();
                if title.is_empty() {
                    continue;
                }
                let owner = columns
                    .get(2)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let status = columns
                    .get(3)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let source = columns
                    .get(4)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());

                deadlines.push((
                    date,
                    MatterDeadlineInfo {
                        date: date.to_string(),
                        title,
                        owner,
                        status,
                        source,
                        is_overdue: date < today,
                    },
                ));
                continue;
            }
        }

        // Fallback parser for checklist-style lines with embedded YYYY-MM-DD.
        if let Some((date, start, end)) = parse_iso_date_token(line) {
            let left = line[..start].trim();
            let right = line[end..].trim();
            let joined = if left.is_empty() {
                right.to_string()
            } else if right.is_empty() {
                left.to_string()
            } else {
                format!("{left} {right}")
            };
            let mut title = joined
                .trim()
                .trim_matches('|')
                .trim_matches('-')
                .trim()
                .to_string();
            if title.is_empty() {
                title = "Untitled deadline".to_string();
            }

            deadlines.push((
                date,
                MatterDeadlineInfo {
                    date: date.to_string(),
                    title,
                    owner: None,
                    status: None,
                    source: None,
                    is_overdue: date < today,
                },
            ));
        }
    }

    deadlines.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    deadlines.into_iter().map(|(_, info)| info).collect()
}

async fn read_matter_deadlines(
    workspace: &Workspace,
    matter_prefix: &str,
    today: NaiveDate,
) -> Result<Vec<MatterDeadlineInfo>, (StatusCode, String)> {
    let path = format!("{matter_prefix}/deadlines/calendar.md");
    match workspace.read(&path).await {
        Ok(doc) => Ok(parse_deadlines_from_calendar(&doc.content, today)),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(Vec::new()),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

pub(crate) async fn read_matter_deadlines_for_matter(
    state: &GatewayState,
    matter_id: &str,
    matter_prefix: &str,
    today: NaiveDate,
) -> Result<Vec<MatterDeadlineInfo>, (StatusCode, String)> {
    if let Some(store) = state.store.as_ref() {
        let records = store
            .list_matter_deadlines(&state.user_id, matter_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        if !records.is_empty() {
            return Ok(records
                .iter()
                .map(deadline_record_to_legacy_info)
                .collect::<Vec<_>>());
        }
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    read_matter_deadlines(workspace.as_ref(), matter_prefix, today).await
}

pub(crate) async fn list_matter_templates(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Result<Vec<MatterTemplateInfo>, (StatusCode, String)> {
    let templates_path = format!("{matter_root}/{matter_id}/templates");
    let entries = match workspace.list(&templates_path).await {
        Ok(entries) => entries,
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Vec::new(),
        Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    };

    let mut templates: Vec<MatterTemplateInfo> = entries
        .into_iter()
        .filter(|entry| !entry.is_directory)
        .filter_map(|entry| {
            let name = entry.path.rsplit('/').next()?.to_string();
            if name.is_empty() {
                return None;
            }
            Some(MatterTemplateInfo {
                id: None,
                matter_id: Some(matter_id.to_string()),
                name,
                path: entry.path,
                variables_json: None,
                updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
            })
        })
        .collect();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

pub(crate) fn document_template_record_to_info(
    matter_root: &str,
    record: crate::db::DocumentTemplateRecord,
) -> MatterTemplateInfo {
    let path = match record.matter_id.as_ref() {
        Some(matter_id) => format!("{matter_root}/{matter_id}/templates/{}", record.name),
        None => format!("templates/shared/{}", record.name),
    };
    MatterTemplateInfo {
        id: Some(record.id.to_string()),
        matter_id: record.matter_id,
        name: record.name,
        path,
        variables_json: Some(record.variables_json),
        updated_at: Some(record.updated_at.to_rfc3339()),
    }
}

pub(crate) fn matter_document_record_to_info(
    record: crate::db::MatterDocumentRecord,
) -> MatterDocumentInfo {
    let fallback_name = record.path.rsplit('/').next().unwrap_or("").to_string();
    MatterDocumentInfo {
        id: Some(record.id.to_string()),
        memory_document_id: Some(record.memory_document_id.to_string()),
        name: fallback_name,
        display_name: Some(record.display_name),
        path: record.path,
        is_dir: false,
        category: Some(record.category.as_str().to_string()),
        updated_at: Some(record.updated_at.to_rfc3339()),
    }
}

pub(crate) async fn backfill_matter_templates_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let Some(workspace) = state.workspace.as_ref() else {
        return Ok(());
    };
    let matter_root = matter_root_for_gateway(state);
    let templates = list_matter_templates(workspace.as_ref(), &matter_root, matter_id).await?;
    for template in templates {
        let doc = workspace
            .read(&template.path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        store
            .upsert_document_template(
                &state.user_id,
                &UpsertDocumentTemplateParams {
                    matter_id: Some(matter_id.to_string()),
                    name: template.name,
                    body: doc.content,
                    variables_json: serde_json::json!([]),
                },
            )
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }
    Ok(())
}

pub(crate) async fn backfill_matter_documents_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let Some(workspace) = state.workspace.as_ref() else {
        return Ok(());
    };

    let matter_prefix = matter_prefix_for_gateway(state, matter_id);
    let docs = list_matter_documents_recursive(workspace.as_ref(), &matter_prefix, false).await?;
    for entry in docs.into_iter().filter(|item| !item.is_dir) {
        let doc = workspace
            .read(&entry.path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let linked = store
            .upsert_matter_document(
                &state.user_id,
                matter_id,
                &UpsertMatterDocumentParams {
                    memory_document_id: doc.id,
                    path: doc.path.clone(),
                    display_name: entry.name.clone(),
                    category: infer_matter_document_category(&entry.path),
                },
            )
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let versions = store
            .list_document_versions(&state.user_id, linked.id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        if versions.is_empty() {
            store
                .create_document_version(
                    &state.user_id,
                    &CreateDocumentVersionParams {
                        matter_document_id: linked.id,
                        label: "initial".to_string(),
                        memory_document_id: doc.id,
                    },
                )
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        }
    }

    Ok(())
}

pub(crate) async fn choose_filing_package_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let candidate = format!("{matter_prefix}/exports/filing-package-{timestamp}{suffix}.md");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to choose a unique filing package destination".to_string(),
    ))
}

pub(crate) async fn read_workspace_matter_metadata_optional(
    workspace: Option<&Arc<Workspace>>,
    matter_root: &str,
    matter_id: &str,
) -> Option<crate::legal::matter::MatterMetadata> {
    let workspace = workspace?;
    let path = format!("{matter_root}/{matter_id}/matter.yaml");
    let doc = workspace.read(&path).await.ok()?;
    serde_yml::from_str(&doc.content).ok()
}

pub(crate) async fn db_matter_to_info(
    state: &GatewayState,
    matter: crate::db::MatterRecord,
) -> MatterInfo {
    let matter_root = matter_root_for_gateway(state);
    let metadata = read_workspace_matter_metadata_optional(
        state.workspace.as_ref(),
        &matter_root,
        &matter.matter_id,
    )
    .await;
    let client_name = if let Some(store) = state.store.as_ref() {
        match store.get_client(&state.user_id, matter.client_id).await {
            Ok(Some(client)) => Some(client.name),
            _ => metadata.as_ref().map(|meta| meta.client.clone()),
        }
    } else {
        metadata.as_ref().map(|meta| meta.client.clone())
    };

    let opened_date = metadata
        .as_ref()
        .and_then(|meta| meta.opened_date.clone())
        .or_else(|| matter.opened_at.map(|dt| dt.date_naive().to_string()));

    MatterInfo {
        id: matter.matter_id.clone(),
        client_id: Some(matter.client_id.to_string()),
        client: client_name,
        status: Some(matter.status.as_str().to_string()),
        stage: matter.stage.clone(),
        confidentiality: metadata.as_ref().map(|meta| meta.confidentiality.clone()),
        team: if let Some(meta) = metadata.as_ref() {
            meta.team.clone()
        } else {
            matter.assigned_to.clone()
        },
        adversaries: metadata
            .as_ref()
            .map(|meta| meta.adversaries.clone())
            .unwrap_or_default(),
        retention: metadata.as_ref().map(|meta| meta.retention.clone()),
        jurisdiction: metadata
            .as_ref()
            .and_then(|meta| meta.jurisdiction.clone())
            .or(matter.jurisdiction.clone()),
        practice_area: metadata
            .as_ref()
            .and_then(|meta| meta.practice_area.clone())
            .or(matter.practice_area.clone()),
        opened_date: opened_date.clone(),
        opened_at: opened_date,
    }
}

pub(crate) async fn ensure_existing_matter_db(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let exists = store
        .get_matter_db(&state.user_id, matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    Ok(())
}

pub(crate) async fn ensure_matter_db_row_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    if store
        .get_matter_db(&state.user_id, matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some()
    {
        return Ok(());
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state);
    let metadata = crate::legal::matter::read_matter_metadata_for_root(
        workspace.as_ref(),
        &matter_root,
        matter_id,
    )
    .await
    .map_err(|err| match err {
        crate::legal::matter::MatterMetadataValidationError::Missing { path } => (
            StatusCode::NOT_FOUND,
            format!("Matter '{}' not found (missing '{}')", matter_id, path),
        ),
        crate::legal::matter::MatterMetadataValidationError::Invalid { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
        }
        crate::legal::matter::MatterMetadataValidationError::Storage { .. } => {
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        }
    })?;

    let client = store
        .upsert_client_by_normalized_name(
            &state.user_id,
            &CreateClientParams {
                name: metadata.client.clone(),
                client_type: ClientType::Entity,
                email: None,
                phone: None,
                address: None,
                notes: None,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let opened_at = parse_optional_datetime("opened_date", metadata.opened_date.clone())?;
    store
        .upsert_matter(
            &state.user_id,
            &UpsertMatterParams {
                matter_id: matter_id.to_string(),
                client_id: client.id,
                status: MatterStatus::Active,
                stage: None,
                practice_area: metadata.practice_area.clone(),
                jurisdiction: metadata.jurisdiction.clone(),
                opened_at,
                closed_at: None,
                assigned_to: metadata.team.clone(),
                custom_fields: serde_json::json!({}),
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok(())
}
pub(crate) fn parse_required_matter_field(
    field_name: &str,
    value: &str,
) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn parse_optional_matter_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

pub(crate) fn parse_optional_matter_field_patch(
    value: Option<Option<String>>,
) -> Option<Option<String>> {
    match value {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(parse_optional_matter_field(Some(raw))),
    }
}

const OPTIONAL_MATTER_FIELD_MAX_CHARS: usize = 256;

pub(crate) fn validate_optional_matter_field_length(
    field_name: &str,
    value: &Option<String>,
) -> Result<(), (StatusCode, String)> {
    if let Some(text) = value
        && text.chars().count() > OPTIONAL_MATTER_FIELD_MAX_CHARS
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' must be at most {} characters",
                field_name, OPTIONAL_MATTER_FIELD_MAX_CHARS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_opened_date(value: &str) -> Result<(), (StatusCode, String)> {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(parsed) if parsed.format("%Y-%m-%d").to_string() == value => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'opened_date' must be in YYYY-MM-DD format".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

pub(crate) fn parse_client_type(value: &str) -> Result<ClientType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "individual" => Ok(ClientType::Individual),
        "entity" => Ok(ClientType::Entity),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'client_type' must be 'individual' or 'entity'".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_status(value: &str) -> Result<MatterStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "intake" => Ok(MatterStatus::Intake),
        "active" => Ok(MatterStatus::Active),
        "pending" => Ok(MatterStatus::Pending),
        "closed" => Ok(MatterStatus::Closed),
        "archived" => Ok(MatterStatus::Archived),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: intake, active, pending, closed, archived".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_task_status(
    value: &str,
) -> Result<MatterTaskStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "todo" => Ok(MatterTaskStatus::Todo),
        "in_progress" => Ok(MatterTaskStatus::InProgress),
        "done" => Ok(MatterTaskStatus::Done),
        "blocked" => Ok(MatterTaskStatus::Blocked),
        "cancelled" => Ok(MatterTaskStatus::Cancelled),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: todo, in_progress, done, blocked, cancelled".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_deadline_type(
    value: &str,
) -> Result<MatterDeadlineType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "court_date" => Ok(MatterDeadlineType::CourtDate),
        "filing" => Ok(MatterDeadlineType::Filing),
        "statute_of_limitations" => Ok(MatterDeadlineType::StatuteOfLimitations),
        "response_due" => Ok(MatterDeadlineType::ResponseDue),
        "discovery_cutoff" => Ok(MatterDeadlineType::DiscoveryCutoff),
        "internal" => Ok(MatterDeadlineType::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'deadline_type' must be one of: court_date, filing, statute_of_limitations, response_due, discovery_cutoff, internal".to_string(),
        )),
    }
}

pub(crate) fn parse_expense_category(value: &str) -> Result<ExpenseCategory, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "filing_fee" => Ok(ExpenseCategory::FilingFee),
        "travel" => Ok(ExpenseCategory::Travel),
        "postage" => Ok(ExpenseCategory::Postage),
        "expert" => Ok(ExpenseCategory::Expert),
        "copying" => Ok(ExpenseCategory::Copying),
        "court_reporter" => Ok(ExpenseCategory::CourtReporter),
        "other" => Ok(ExpenseCategory::Other),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: filing_fee, travel, postage, expert, copying, court_reporter, other".to_string(),
        )),
    }
}

pub(crate) fn parse_date_only(
    field_name: &str,
    raw: &str,
) -> Result<NaiveDate, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        )
    })?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        ));
    }
    Ok(parsed)
}

pub(crate) fn parse_decimal_field(
    field_name: &str,
    raw: &str,
) -> Result<Decimal, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let decimal = value.parse::<Decimal>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid decimal number", field_name),
        )
    })?;
    if decimal <= Decimal::ZERO {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be greater than 0", field_name),
        ));
    }
    Ok(decimal)
}

pub(crate) fn parse_optional_decimal_field(
    field_name: &str,
    raw: Option<String>,
) -> Result<Option<Decimal>, (StatusCode, String)> {
    match parse_optional_matter_field(raw) {
        Some(value) => parse_decimal_field(field_name, &value).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn parse_matter_document_category(
    value: Option<&str>,
) -> Result<MatterDocumentCategory, (StatusCode, String)> {
    let raw = value.unwrap_or("internal").trim().to_ascii_lowercase();
    match raw.as_str() {
        "pleading" => Ok(MatterDocumentCategory::Pleading),
        "correspondence" => Ok(MatterDocumentCategory::Correspondence),
        "contract" => Ok(MatterDocumentCategory::Contract),
        "filing" => Ok(MatterDocumentCategory::Filing),
        "evidence" => Ok(MatterDocumentCategory::Evidence),
        "internal" | "" => Ok(MatterDocumentCategory::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: pleading, correspondence, contract, filing, evidence, internal".to_string(),
        )),
    }
}

fn infer_matter_document_category(path: &str) -> MatterDocumentCategory {
    let lower = path.to_ascii_lowercase();
    if lower.contains("/filing") || lower.contains("/pleading") {
        MatterDocumentCategory::Filing
    } else if lower.contains("/evidence") {
        MatterDocumentCategory::Evidence
    } else if lower.contains("/contract") || lower.contains("/agreement") {
        MatterDocumentCategory::Contract
    } else if lower.contains("/correspondence") || lower.contains("/communication") {
        MatterDocumentCategory::Correspondence
    } else {
        MatterDocumentCategory::Internal
    }
}

pub(crate) fn normalize_reminder_days(values: &[i32]) -> Result<Vec<i32>, (StatusCode, String)> {
    use std::collections::BTreeSet;

    if values.len() > MAX_DEADLINE_REMINDERS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'reminder_days' supports at most {} values",
                MAX_DEADLINE_REMINDERS
            ),
        ));
    }

    let mut unique = BTreeSet::new();
    for day in values {
        if *day < 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "'reminder_days' values must be >= 0".to_string(),
            ));
        }
        if *day > MAX_DEADLINE_REMINDER_DAYS {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "'reminder_days' values must be <= {}",
                    MAX_DEADLINE_REMINDER_DAYS
                ),
            ));
        }
        unique.insert(*day);
    }

    Ok(unique.into_iter().collect())
}

pub(crate) fn parse_datetime_value(
    field: &str,
    raw: &str,
) -> Result<DateTime<Utc>, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' cannot be empty", field),
        ));
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(0, 0, 0)
    {
        return Ok(dt.and_utc());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&Utc));
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("'{}' must be YYYY-MM-DD or RFC3339 datetime", field),
    ))
}

pub(crate) fn parse_optional_datetime(
    field: &str,
    raw: Option<String>,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_datetime_value(field, &raw).map(Some)
}

pub(crate) fn parse_optional_datetime_patch(
    field: &str,
    raw: Option<Option<String>>,
) -> Result<Option<Option<DateTime<Utc>>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    if raw.trim().is_empty() {
        return Ok(Some(None));
    }
    Ok(Some(Some(parse_datetime_value(field, &raw)?)))
}

pub(crate) fn parse_uuid(value: &str, field: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid UUID", field),
        )
    })
}

pub(crate) fn parse_optional_uuid_field(
    value: Option<String>,
    field: &str,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_uuid(trimmed, field).map(Some)
}

pub(crate) fn parse_optional_uuid_patch_field(
    value: Option<Option<String>>,
    field: &str,
) -> Result<Option<Option<Uuid>>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Some(None));
    }
    parse_uuid(trimmed, field).map(|uuid| Some(Some(uuid)))
}

pub(crate) fn deadline_record_to_info(
    record: crate::db::MatterDeadlineRecord,
) -> MatterDeadlineRecordInfo {
    let today = Utc::now().date_naive();
    let due_date = record.due_at.date_naive();
    MatterDeadlineRecordInfo {
        id: record.id.to_string(),
        title: record.title,
        deadline_type: record.deadline_type.as_str().to_string(),
        due_at: record.due_at.to_rfc3339(),
        completed_at: record.completed_at.map(|value| value.to_rfc3339()),
        reminder_days: record.reminder_days,
        rule_ref: record.rule_ref,
        computed_from: record.computed_from.map(|value| value.to_string()),
        task_id: record.task_id.map(|value| value.to_string()),
        is_overdue: record.completed_at.is_none() && due_date < today,
        days_until_due: due_date.signed_duration_since(today).num_days(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn deadline_record_to_legacy_info(record: &crate::db::MatterDeadlineRecord) -> MatterDeadlineInfo {
    let today = Utc::now().date_naive();
    let due_date = record.due_at.date_naive();
    let status = if record.completed_at.is_some() {
        Some("completed".to_string())
    } else {
        Some("open".to_string())
    };
    MatterDeadlineInfo {
        date: due_date.to_string(),
        title: record.title.clone(),
        owner: None,
        status,
        source: record.rule_ref.clone(),
        is_overdue: record.completed_at.is_none() && due_date < today,
    }
}

pub(crate) fn deadline_compute_preview_from_params(
    params: &CreateMatterDeadlineParams,
) -> MatterDeadlineComputePreview {
    let today = Utc::now().date_naive();
    let due_date = params.due_at.date_naive();
    MatterDeadlineComputePreview {
        title: params.title.clone(),
        deadline_type: params.deadline_type.as_str().to_string(),
        due_at: params.due_at.to_rfc3339(),
        reminder_days: params.reminder_days.clone(),
        rule_ref: params.rule_ref.clone(),
        computed_from: params.computed_from.map(|value| value.to_string()),
        task_id: params.task_id.map(|value| value.to_string()),
        is_overdue: due_date < today,
        days_until_due: due_date.signed_duration_since(today).num_days(),
    }
}

pub(crate) fn deadline_reminder_prefix(matter_id: &str, deadline_id: Uuid) -> String {
    format!("deadline-reminder-{matter_id}-{deadline_id}-")
}

fn deadline_reminder_name(matter_id: &str, deadline_id: Uuid, reminder_days: i32) -> String {
    format!(
        "{}{}",
        deadline_reminder_prefix(matter_id, deadline_id),
        reminder_days
    )
}

fn deadline_reminder_schedule(run_at: DateTime<Utc>) -> String {
    format!(
        "{} {} {} {} {} *",
        run_at.second(),
        run_at.minute(),
        run_at.hour(),
        run_at.day(),
        run_at.month()
    )
}

pub(crate) async fn disable_deadline_reminder_routines(
    state: &GatewayState,
    matter_id: &str,
    deadline_id: Uuid,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let prefix = deadline_reminder_prefix(matter_id, deadline_id);
    let routines = store
        .list_routines(&state.user_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    for mut routine in routines {
        if !routine.name.starts_with(&prefix) || !routine.enabled {
            continue;
        }
        routine.enabled = false;
        routine.next_fire_at = None;
        store
            .update_routine(&routine)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }

    Ok(())
}

pub(crate) async fn sync_deadline_reminder_routines_for_record(
    state: &GatewayState,
    record: &crate::db::MatterDeadlineRecord,
) -> Result<(), (StatusCode, String)> {
    disable_deadline_reminder_routines(state, &record.matter_id, record.id).await?;
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };

    if record.completed_at.is_some() || record.reminder_days.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    for reminder_days in &record.reminder_days {
        let run_at = record.due_at - chrono::Duration::days(i64::from(*reminder_days));
        if run_at <= now {
            continue;
        }

        let name = deadline_reminder_name(&record.matter_id, record.id, *reminder_days);
        let schedule = deadline_reminder_schedule(run_at);
        let next_fire = crate::agent::routine::next_cron_fire(&schedule)
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let prompt = format!(
            "Matter `{}` deadline reminder: \"{}\" is due on {} ({} days remaining). Provide a concise reminder and immediate next action.",
            record.matter_id,
            record.title,
            record.due_at.date_naive(),
            reminder_days
        );
        let state_json = serde_json::json!({
            "one_shot": true,
            "deadline_id": record.id,
            "matter_id": record.matter_id,
            "reminder_days": reminder_days,
        });

        if let Some(mut existing) = store
            .get_routine_by_name(&state.user_id, &name)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        {
            existing.enabled = true;
            existing.trigger = crate::agent::routine::Trigger::Cron { schedule };
            existing.action = crate::agent::routine::RoutineAction::Lightweight {
                prompt: prompt.clone(),
                context_paths: vec![matter_metadata_path_for_gateway(state, &record.matter_id)],
                max_tokens: 300,
            };
            existing.next_fire_at = next_fire;
            existing.state = state_json.clone();
            store
                .update_routine(&existing)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
            continue;
        }

        let routine = crate::agent::routine::Routine {
            id: Uuid::new_v4(),
            name,
            description: format!(
                "One-shot reminder {} day(s) before deadline '{}'",
                reminder_days, record.title
            ),
            user_id: state.user_id.clone(),
            enabled: true,
            trigger: crate::agent::routine::Trigger::Cron { schedule },
            action: crate::agent::routine::RoutineAction::Lightweight {
                prompt,
                context_paths: vec![matter_metadata_path_for_gateway(state, &record.matter_id)],
                max_tokens: 300,
            },
            guardrails: crate::agent::routine::RoutineGuardrails::default(),
            notify: crate::agent::routine::NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: next_fire,
            run_count: 0,
            consecutive_failures: 0,
            state: state_json,
            created_at: now,
            updated_at: now,
        };
        store
            .create_routine(&routine)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }

    Ok(())
}

pub(crate) fn parse_uuid_list(
    values: &[String],
    field: &str,
) -> Result<Vec<Uuid>, (StatusCode, String)> {
    values
        .iter()
        .map(|value| parse_uuid(value, field))
        .collect()
}

pub(crate) fn client_record_to_info(client: crate::db::ClientRecord) -> ClientInfo {
    ClientInfo {
        id: client.id.to_string(),
        name: client.name,
        client_type: client.client_type.as_str().to_string(),
        email: client.email,
        phone: client.phone,
        address: client.address,
        notes: client.notes,
        created_at: client.created_at.to_rfc3339(),
        updated_at: client.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_task_record_to_info(task: crate::db::MatterTaskRecord) -> MatterTaskInfo {
    MatterTaskInfo {
        id: task.id.to_string(),
        title: task.title,
        description: task.description,
        status: task.status.as_str().to_string(),
        assignee: task.assignee,
        due_at: task.due_at.map(|dt| dt.to_rfc3339()),
        blocked_by: task
            .blocked_by
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_note_record_to_info(note: crate::db::MatterNoteRecord) -> MatterNoteInfo {
    MatterNoteInfo {
        id: note.id.to_string(),
        author: note.author,
        body: note.body,
        pinned: note.pinned,
        created_at: note.created_at.to_rfc3339(),
        updated_at: note.updated_at.to_rfc3339(),
    }
}

pub(crate) fn time_entry_record_to_info(entry: crate::db::TimeEntryRecord) -> TimeEntryInfo {
    TimeEntryInfo {
        id: entry.id.to_string(),
        timekeeper: entry.timekeeper,
        description: entry.description,
        hours: entry.hours.to_string(),
        hourly_rate: entry.hourly_rate.map(|value| value.to_string()),
        entry_date: entry.entry_date.to_string(),
        billable: entry.billable,
        billed_invoice_id: entry.billed_invoice_id,
        created_at: entry.created_at.to_rfc3339(),
        updated_at: entry.updated_at.to_rfc3339(),
    }
}

pub(crate) fn expense_entry_record_to_info(
    entry: crate::db::ExpenseEntryRecord,
) -> ExpenseEntryInfo {
    ExpenseEntryInfo {
        id: entry.id.to_string(),
        submitted_by: entry.submitted_by,
        description: entry.description,
        amount: entry.amount.to_string(),
        category: entry.category.as_str().to_string(),
        entry_date: entry.entry_date.to_string(),
        receipt_path: entry.receipt_path,
        billable: entry.billable,
        billed_invoice_id: entry.billed_invoice_id,
        created_at: entry.created_at.to_rfc3339(),
        updated_at: entry.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_time_summary_to_response(
    summary: crate::db::MatterTimeSummary,
) -> MatterTimeSummaryResponse {
    MatterTimeSummaryResponse {
        total_hours: summary.total_hours.to_string(),
        billable_hours: summary.billable_hours.to_string(),
        unbilled_hours: summary.unbilled_hours.to_string(),
        total_expenses: summary.total_expenses.to_string(),
        billable_expenses: summary.billable_expenses.to_string(),
        unbilled_expenses: summary.unbilled_expenses.to_string(),
    }
}

pub(crate) fn invoice_record_to_info(invoice: InvoiceRecord) -> InvoiceInfo {
    InvoiceInfo {
        id: invoice.id.to_string(),
        matter_id: invoice.matter_id,
        invoice_number: invoice.invoice_number,
        status: invoice.status.as_str().to_string(),
        issued_date: invoice.issued_date.map(|value| value.to_string()),
        due_date: invoice.due_date.map(|value| value.to_string()),
        subtotal: invoice.subtotal.to_string(),
        tax: invoice.tax.to_string(),
        total: invoice.total.to_string(),
        paid_amount: invoice.paid_amount.to_string(),
        notes: invoice.notes,
        created_at: invoice.created_at.to_rfc3339(),
        updated_at: invoice.updated_at.to_rfc3339(),
    }
}

pub(crate) fn invoice_draft_to_info(invoice: &crate::db::CreateInvoiceParams) -> InvoiceDraftInfo {
    InvoiceDraftInfo {
        matter_id: invoice.matter_id.clone(),
        invoice_number: invoice.invoice_number.clone(),
        status: invoice.status.as_str().to_string(),
        due_date: invoice.due_date.map(|value| value.to_string()),
        subtotal: invoice.subtotal.to_string(),
        tax: invoice.tax.to_string(),
        total: invoice.total.to_string(),
        notes: invoice.notes.clone(),
    }
}

pub(crate) fn invoice_line_item_record_to_info(item: InvoiceLineItemRecord) -> InvoiceLineItemInfo {
    InvoiceLineItemInfo {
        id: item.id.to_string(),
        description: item.description,
        quantity: item.quantity.to_string(),
        unit_price: item.unit_price.to_string(),
        amount: item.amount.to_string(),
        time_entry_id: item.time_entry_id.map(|value| value.to_string()),
        expense_entry_id: item.expense_entry_id.map(|value| value.to_string()),
        sort_order: item.sort_order,
    }
}

pub(crate) fn invoice_line_item_params_to_info(
    item: &crate::db::CreateInvoiceLineItemParams,
) -> InvoiceLineItemInfo {
    InvoiceLineItemInfo {
        id: "draft".to_string(),
        description: item.description.clone(),
        quantity: item.quantity.to_string(),
        unit_price: item.unit_price.to_string(),
        amount: item.amount.to_string(),
        time_entry_id: item.time_entry_id.map(|value| value.to_string()),
        expense_entry_id: item.expense_entry_id.map(|value| value.to_string()),
        sort_order: item.sort_order,
    }
}

pub(crate) fn trust_ledger_entry_record_to_info(
    entry: TrustLedgerEntryRecord,
) -> TrustLedgerEntryInfo {
    TrustLedgerEntryInfo {
        id: entry.id.to_string(),
        matter_id: entry.matter_id,
        entry_type: entry.entry_type.as_str().to_string(),
        amount: entry.amount.to_string(),
        balance_after: entry.balance_after.to_string(),
        description: entry.description,
        invoice_id: entry.invoice_id.map(|value| value.to_string()),
        recorded_by: entry.recorded_by,
        created_at: entry.created_at.to_rfc3339(),
    }
}

pub(crate) fn audit_event_record_to_info(
    event: crate::db::AuditEventRecord,
) -> LegalAuditEventInfo {
    LegalAuditEventInfo {
        id: event.id.to_string(),
        ts: event.created_at.to_rfc3339(),
        event_type: event.event_type,
        actor: event.actor,
        matter_id: event.matter_id,
        severity: event.severity.as_str().to_string(),
        details: event.details,
    }
}

fn validate_intake_party_name(field_name: &str, value: &str) -> Result<(), (StatusCode, String)> {
    if value.chars().count() > MAX_INTAKE_CONFLICT_PARTY_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' entries must be at most {} characters",
                field_name, MAX_INTAKE_CONFLICT_PARTY_CHARS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_intake_party_list(
    field_name: &str,
    values: &[String],
) -> Result<(), (StatusCode, String)> {
    if values.len() > MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' may include at most {} names",
                field_name, MAX_INTAKE_CONFLICT_PARTIES
            ),
        ));
    }
    for value in values {
        validate_intake_party_name(field_name, value)?;
    }
    Ok(())
}

pub(crate) fn list_matters_root_entries(
    result: Result<Vec<crate::workspace::WorkspaceEntry>, crate::error::WorkspaceError>,
) -> Result<Vec<crate::workspace::WorkspaceEntry>, (StatusCode, String)> {
    match result {
        Ok(entries) => Ok(entries),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(Vec::new()),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}
