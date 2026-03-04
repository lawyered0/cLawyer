//! Matter document/dashboard/template handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::{NaiveDate, Utc};

use crate::channels::web::server::MatterDocumentsQuery;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{CreateDocumentVersionParams, MatterDocumentCategory, UpsertMatterDocumentParams};

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/matters/{id}/documents", get(matter_documents_handler))
        .route("/api/matters/{id}/dashboard", get(matter_dashboard_handler))
        .route("/api/matters/{id}/templates", get(matter_templates_handler))
        .route(
            "/api/matters/{id}/templates/apply",
            post(matter_template_apply_handler),
        )
        .route(
            "/api/matters/{id}/exports/retrieval-packet",
            post(matter_retrieval_export_handler),
        )
        .route("/api/documents/generate", post(documents_generate_handler))
        .route(
            "/api/matters/{id}/filing-package",
            post(matter_filing_package_handler),
        )
}

pub(crate) async fn matter_documents_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<MatterDocumentsQuery>,
) -> Result<Json<MatterDocumentsResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let include_templates = query.include_templates.unwrap_or(false);

    let documents = if state.store.is_some() {
        crate::channels::web::server::ensure_matter_db_row_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        crate::channels::web::server::backfill_matter_documents_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        let store = state.store.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Database not available".to_string(),
        ))?;
        let mut docs = store
            .list_matter_documents_db(&state.user_id, &matter_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
            .into_iter()
            .map(crate::channels::web::server::matter_document_record_to_info)
            .collect::<Vec<_>>();
        if include_templates {
            crate::channels::web::server::backfill_matter_templates_from_workspace(
                state.as_ref(),
                &matter_id,
            )
            .await?;
            let templates = store
                .list_document_templates(&state.user_id, Some(&matter_id))
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
            docs.extend(
                templates
                    .into_iter()
                    .map(|record| {
                        crate::channels::web::server::document_template_record_to_info(
                            &matter_root,
                            record,
                        )
                    })
                    .map(|template| MatterDocumentInfo {
                        id: template.id,
                        memory_document_id: None,
                        name: template.name.clone(),
                        display_name: Some(template.name),
                        path: template.path,
                        is_dir: false,
                        category: Some("template".to_string()),
                        updated_at: template.updated_at,
                    }),
            );
            docs.sort_by(|a, b| a.path.cmp(&b.path));
        }
        docs
    } else {
        let matter_prefix = format!("{matter_root}/{matter_id}");
        crate::channels::web::server::list_matter_documents_recursive(
            workspace.as_ref(),
            &matter_prefix,
            include_templates,
        )
        .await?
    };

    Ok(Json(MatterDocumentsResponse {
        matter_id,
        documents,
    }))
}

pub(crate) async fn matter_dashboard_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDashboardResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let matter_prefix = format!("{matter_root}/{matter_id}");
    let docs = crate::channels::web::server::list_matter_documents_recursive(
        workspace.as_ref(),
        &matter_prefix,
        false,
    )
    .await?;
    let templates = crate::channels::web::server::list_matter_templates(
        workspace.as_ref(),
        &matter_root,
        &matter_id,
    )
    .await?;
    let today = Utc::now().date_naive();
    let deadlines = crate::channels::web::server::read_matter_deadlines_for_matter(
        state.as_ref(),
        &matter_id,
        &matter_prefix,
        today,
    )
    .await?;

    let document_count = docs.iter().filter(|doc| !doc.is_dir).count();
    let draft_prefix = format!("{matter_prefix}/drafts/");
    let draft_count = docs
        .iter()
        .filter(|doc| !doc.is_dir && doc.path.starts_with(&draft_prefix))
        .count();

    let checklist_files = [
        format!("{matter_prefix}/workflows/intake_checklist.md"),
        format!("{matter_prefix}/workflows/review_and_filing_checklist.md"),
    ];
    let mut checklist_completed = 0usize;
    let mut checklist_total = 0usize;
    for path in checklist_files {
        match workspace.read(&path).await {
            Ok(doc) => {
                let (completed, total) =
                    crate::channels::web::server::checklist_completion_from_markdown(&doc.content);
                checklist_completed += completed;
                checklist_total += total;
            }
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {}
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    let mut overdue_deadlines = 0usize;
    let mut upcoming_deadlines_14d = 0usize;
    let mut next_deadline: Option<(NaiveDate, MatterDeadlineInfo)> = None;
    for deadline in deadlines {
        let Ok(date) = NaiveDate::parse_from_str(&deadline.date, "%Y-%m-%d") else {
            continue;
        };
        if date < today {
            overdue_deadlines += 1;
            continue;
        }
        let days_until = date.signed_duration_since(today).num_days();
        if days_until <= 14 {
            upcoming_deadlines_14d += 1;
        }
        if next_deadline
            .as_ref()
            .is_none_or(|(existing, _)| date < *existing)
        {
            next_deadline = Some((date, deadline));
        }
    }

    Ok(Json(MatterDashboardResponse {
        matter_id,
        document_count,
        template_count: templates.len(),
        draft_count,
        checklist_completed,
        checklist_total,
        overdue_deadlines,
        upcoming_deadlines_14d,
        next_deadline: next_deadline.map(|(_, item)| item),
    }))
}

pub(crate) async fn matter_templates_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTemplatesResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let templates = if let Some(store) = state.store.as_ref() {
        crate::channels::web::server::ensure_matter_db_row_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        crate::channels::web::server::backfill_matter_templates_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        store
            .list_document_templates(&state.user_id, Some(&matter_id))
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
            .into_iter()
            .map(|record| {
                crate::channels::web::server::document_template_record_to_info(&matter_root, record)
            })
            .collect::<Vec<_>>()
    } else {
        crate::channels::web::server::list_matter_templates(
            workspace.as_ref(),
            &matter_root,
            &matter_id,
        )
        .await?
    };

    Ok(Json(MatterTemplatesResponse {
        matter_id,
        templates,
    }))
}

pub(crate) async fn matter_template_apply_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<MatterTemplateApplyRequest>,
) -> Result<(StatusCode, Json<MatterTemplateApplyResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let matter_prefix = format!("{matter_root}/{matter_id}");
    let template_name = crate::channels::web::server::parse_template_name(&req.template_name)?;

    let template_body = if let Some(store) = state.store.as_ref() {
        crate::channels::web::server::ensure_matter_db_row_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        crate::channels::web::server::backfill_matter_templates_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        let template = store
            .get_document_template_by_name(&state.user_id, Some(&matter_id), &template_name)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
            .ok_or((
                StatusCode::NOT_FOUND,
                format!("Template '{}' not found", template_name),
            ))?;
        template.body
    } else {
        let template_path = format!("{matter_prefix}/templates/{template_name}");
        workspace
            .read(&template_path)
            .await
            .map_err(|err| match err {
                crate::error::WorkspaceError::DocumentNotFound { .. } => (
                    StatusCode::NOT_FOUND,
                    format!("Template '{}' not found", template_name),
                ),
                other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
            })?
            .content
    };

    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let destination = crate::channels::web::server::choose_template_apply_destination(
        workspace.as_ref(),
        &matter_prefix,
        &template_name,
        &timestamp,
    )
    .await?;

    let written = workspace
        .write(&destination, &template_body)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if let Some(store) = state.store.as_ref() {
        let linked = store
            .upsert_matter_document(
                &state.user_id,
                &matter_id,
                &UpsertMatterDocumentParams {
                    memory_document_id: written.id,
                    path: written.path.clone(),
                    display_name: template_name.clone(),
                    category: MatterDocumentCategory::Internal,
                },
            )
            .await;
        let linked = match linked {
            Ok(value) => value,
            Err(err) => {
                let _ = workspace.delete(&destination).await;
                return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
            }
        };
        if let Err(err) = store
            .create_document_version(
                &state.user_id,
                &CreateDocumentVersionParams {
                    matter_document_id: linked.id,
                    label: "draft".to_string(),
                    memory_document_id: written.id,
                },
            )
            .await
        {
            let _ = store
                .delete_matter_document(&state.user_id, &matter_id, linked.id)
                .await;
            let _ = workspace.delete(&destination).await;
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(MatterTemplateApplyResponse {
            path: destination,
            status: "created",
        }),
    ))
}

pub(crate) async fn matter_retrieval_export_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<MatterRetrievalExportRequest>>,
) -> Result<(StatusCode, Json<MatterRetrievalExportResponse>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let legal = crate::channels::web::server::legal_config_for_gateway(state.as_ref());
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;

    let unredacted = body.as_ref().is_some_and(|Json(value)| value.unredacted);

    let result = crate::legal::backup::export_matter_retrieval_packet(
        store.as_ref(),
        workspace.as_ref(),
        &state.user_id,
        &matter_id,
        &crate::legal::backup::MatterRetrievalExportOptions {
            redacted: !unredacted,
            matter_root: legal.matter_root.clone(),
        },
        Some(&legal.redaction),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "matter_retrieval_exported",
        state.user_id.as_str(),
        Some(matter_id.as_str()),
        if unredacted {
            crate::db::AuditSeverity::Warn
        } else {
            crate::db::AuditSeverity::Info
        },
        serde_json::json!({
            "output_dir": result.output_dir,
            "redacted": result.redacted,
            "file_count": result.files.len(),
            "warning": result.warning,
        }),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(MatterRetrievalExportResponse {
            matter_id: result.matter_id,
            output_dir: result.output_dir,
            redacted: result.redacted,
            files: result.files,
            warning: result.warning,
        }),
    ))
}

pub(crate) async fn documents_generate_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<GenerateDocumentRequest>,
) -> Result<(StatusCode, Json<GenerateDocumentResponse>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &req.matter_id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;
    crate::channels::web::server::backfill_matter_templates_from_workspace(
        state.as_ref(),
        &matter_id,
    )
    .await?;

    let template_id =
        crate::channels::web::server::parse_uuid(req.template_id.trim(), "template_id")?;
    let template = store
        .get_document_template(&state.user_id, template_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Template not found".to_string()))?;
    if let Some(ref template_matter) = template.matter_id
        && template_matter != &matter_id
    {
        return Err((
            StatusCode::NOT_FOUND,
            "Template not available for this matter".to_string(),
        ));
    }

    let matter = store
        .get_matter_db(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;
    let client = store
        .get_client(&state.user_id, matter.client_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::UNPROCESSABLE_ENTITY,
            "Matter is missing an associated client record".to_string(),
        ))?;

    let extra = if req.extra.is_object() {
        req.extra
    } else {
        serde_json::json!({})
    };
    let context = crate::legal::docgen::build_context(&matter, &client, Some(&extra));
    let rendered = crate::legal::docgen::render_template(&template.body, &context)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;

    let category =
        crate::channels::web::server::parse_matter_document_category(req.category.as_deref())?;
    let display_name = crate::channels::web::server::parse_optional_matter_field(req.display_name)
        .unwrap_or_else(|| template.name.clone());
    let label = crate::channels::web::server::parse_optional_matter_field(req.label)
        .unwrap_or_else(|| "draft".to_string());
    crate::channels::web::server::validate_optional_matter_field_length(
        "display_name",
        &Some(display_name.clone()),
    )?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "label",
        &Some(label.clone()),
    )?;

    let matter_prefix = format!("{matter_root}/{matter_id}");
    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let destination = crate::channels::web::server::choose_generated_document_destination(
        workspace.as_ref(),
        &matter_prefix,
        &template.name,
        &timestamp,
    )
    .await?;

    let written = workspace
        .write(&destination, &rendered)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let linked = match store
        .upsert_matter_document(
            &state.user_id,
            &matter_id,
            &UpsertMatterDocumentParams {
                memory_document_id: written.id,
                path: written.path.clone(),
                display_name: display_name.clone(),
                category,
            },
        )
        .await
    {
        Ok(linked) => linked,
        Err(err) => {
            let _ = workspace.delete(&destination).await;
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
        }
    };

    let version = match store
        .create_document_version(
            &state.user_id,
            &CreateDocumentVersionParams {
                matter_document_id: linked.id,
                label: label.clone(),
                memory_document_id: written.id,
            },
        )
        .await
    {
        Ok(version) => version,
        Err(err) => {
            let _ = store
                .delete_matter_document(&state.user_id, &matter_id, linked.id)
                .await;
            let _ = workspace.delete(&destination).await;
            return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
        }
    };

    Ok((
        StatusCode::CREATED,
        Json(GenerateDocumentResponse {
            matter_document_id: linked.id.to_string(),
            memory_document_id: linked.memory_document_id.to_string(),
            path: linked.path,
            display_name: linked.display_name,
            category: linked.category.as_str().to_string(),
            version_number: version.version_number,
            label: version.label,
        }),
    ))
}

pub(crate) async fn matter_filing_package_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<MatterFilingPackageResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let matter_prefix = format!("{matter_root}/{matter_id}");
    let generated_at = Utc::now();
    let timestamp = generated_at.format("%Y%m%d-%H%M%S").to_string();
    let destination = crate::channels::web::server::choose_filing_package_destination(
        workspace.as_ref(),
        &matter_prefix,
        &timestamp,
    )
    .await?;

    let metadata = crate::legal::matter::read_matter_metadata_for_root(
        workspace.as_ref(),
        &matter_root,
        &matter_id,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let docs = crate::channels::web::server::list_matter_documents_recursive(
        workspace.as_ref(),
        &matter_prefix,
        true,
    )
    .await?;
    let templates = crate::channels::web::server::list_matter_templates(
        workspace.as_ref(),
        &matter_root,
        &matter_id,
    )
    .await?;
    let today = generated_at.date_naive();
    let deadlines = crate::channels::web::server::read_matter_deadlines_for_matter(
        state.as_ref(),
        &matter_id,
        &matter_prefix,
        today,
    )
    .await?;

    let checklist_files = [
        format!("{matter_prefix}/workflows/intake_checklist.md"),
        format!("{matter_prefix}/workflows/review_and_filing_checklist.md"),
    ];
    let mut checklist_completed = 0usize;
    let mut checklist_total = 0usize;
    for path in checklist_files {
        match workspace.read(&path).await {
            Ok(doc) => {
                let (completed, total) =
                    crate::channels::web::server::checklist_completion_from_markdown(&doc.content);
                checklist_completed += completed;
                checklist_total += total;
            }
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {}
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    let mut package = String::new();
    package.push_str("# Filing Package Index\n\n");
    package.push_str(&format!("Matter: `{}`\n", matter_id));
    package.push_str(&format!("Client: {}\n", metadata.client));
    package.push_str(&format!("Confidentiality: {}\n", metadata.confidentiality));
    package.push_str(&format!("Generated: {}\n\n", generated_at.to_rfc3339()));

    let file_docs: Vec<&MatterDocumentInfo> = docs.iter().filter(|doc| !doc.is_dir).collect();
    let draft_prefix = format!("{matter_prefix}/drafts/");
    let draft_count = file_docs
        .iter()
        .filter(|doc| doc.path.starts_with(&draft_prefix))
        .count();
    let overdue_deadlines = deadlines.iter().filter(|item| item.is_overdue).count();
    let upcoming_deadlines_14d = deadlines
        .iter()
        .filter_map(|item| {
            NaiveDate::parse_from_str(&item.date, "%Y-%m-%d")
                .ok()
                .map(|date| date.signed_duration_since(today).num_days())
        })
        .filter(|days| (0..=14).contains(days))
        .count();

    package.push_str("## Workflow Scorecard\n\n");
    package.push_str(&format!("- Documents: {}\n", file_docs.len()));
    package.push_str(&format!("- Drafts: {}\n", draft_count));
    package.push_str(&format!("- Templates: {}\n", templates.len()));
    package.push_str(&format!(
        "- Checklist completion: {}/{}\n",
        checklist_completed, checklist_total
    ));
    package.push_str(&format!("- Overdue deadlines: {}\n", overdue_deadlines));
    package.push_str(&format!(
        "- Upcoming deadlines (14d): {}\n\n",
        upcoming_deadlines_14d
    ));

    package.push_str("## Deadlines Snapshot\n\n");
    if deadlines.is_empty() {
        package.push_str("- None parsed from `deadlines/calendar.md`.\n\n");
    } else {
        package.push_str("| Date | Event | Owner | Status | Source |\n");
        package.push_str("|---|---|---|---|---|\n");
        for deadline in &deadlines {
            package.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                deadline.date,
                deadline.title.replace('|', "\\|"),
                deadline
                    .owner
                    .clone()
                    .unwrap_or_default()
                    .replace('|', "\\|"),
                deadline
                    .status
                    .clone()
                    .unwrap_or_default()
                    .replace('|', "\\|"),
                deadline
                    .source
                    .clone()
                    .unwrap_or_default()
                    .replace('|', "\\|"),
            ));
        }
        package.push('\n');
    }

    package.push_str("## Document Inventory\n\n");
    if file_docs.is_empty() {
        package.push_str("- No documents found.\n\n");
    } else {
        for doc in &file_docs {
            package.push_str(&format!("- `{}`\n", doc.path));
        }
        package.push('\n');
    }

    package.push_str("## Template Inventory\n\n");
    if templates.is_empty() {
        package.push_str("- No templates found.\n");
    } else {
        for template in &templates {
            package.push_str(&format!("- `{}`\n", template.path));
        }
    }

    workspace
        .write(&destination, &package)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(MatterFilingPackageResponse {
            matter_id,
            path: destination,
            generated_at: generated_at.to_rfc3339(),
            status: "created",
        }),
    ))
}
