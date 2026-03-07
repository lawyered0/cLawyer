//! Matter/document/template/deadline helpers for web handlers.

use std::path::Path as FsPath;
use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    ClientType, CreateClientParams, CreateDocumentVersionParams, CreateMatterDeadlineParams,
    MatterMemberRole, MatterStatus, UpsertDocumentTemplateParams, UpsertMatterDocumentParams,
    UpsertMatterParams,
};
use crate::workspace::Workspace;

use super::legal::{
    matter_metadata_path_for_gateway, matter_prefix_for_gateway, matter_root_for_gateway,
};
use super::parsing::{infer_matter_document_category, parse_optional_datetime};

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
                readiness_state: None,
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
        readiness_state: Some(record.readiness_state.as_str().to_string()),
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
                    readiness_state: None,
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
pub(crate) fn list_matters_root_entries(
    result: Result<Vec<crate::workspace::WorkspaceEntry>, crate::error::WorkspaceError>,
) -> Result<Vec<crate::workspace::WorkspaceEntry>, (StatusCode, String)> {
    match result {
        Ok(entries) => Ok(entries),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(Vec::new()),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

// ==================== RBAC Guard ====================

/// Role hierarchy rank — higher value = more permissive.
fn role_rank(role: &MatterMemberRole) -> u8 {
    match role {
        MatterMemberRole::Viewer => 1,
        MatterMemberRole::Collaborator => 2,
        MatterMemberRole::Owner => 3,
    }
}

/// Verify that `requesting_user_id` holds at least `minimum_role` for the matter.
///
/// When no store is configured (workspace-only mode), access is always granted
/// as Owner — there are no other users to gate against.
///
/// Returns the effective [`MatterMemberRole`] on success, or an appropriate
/// HTTP status code (`FORBIDDEN` / `INTERNAL_SERVER_ERROR`) on failure.
pub async fn require_matter_access(
    store: &Option<Arc<dyn crate::db::Database>>,
    matter_owner_user_id: &str,
    matter_id: &str,
    requesting_user_id: &str,
    minimum_role: MatterMemberRole,
) -> Result<MatterMemberRole, StatusCode> {
    let Some(store) = store.as_ref() else {
        // Single-user workspace mode — no other users exist.
        return Ok(MatterMemberRole::Owner);
    };
    let role = store
        .check_matter_access(matter_owner_user_id, matter_id, requesting_user_id)
        .await
        .map_err(|e| {
            tracing::error!("check_matter_access failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::FORBIDDEN)?;
    if role_rank(&role) < role_rank(&minimum_role) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(role)
}
