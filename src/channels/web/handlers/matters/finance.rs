//! Matter finance handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::channels::web::auth::RequestPrincipal;
use crate::channels::web::handlers::helpers::matter::require_matter_access;
use crate::channels::web::server::MatterInvoicesQuery;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    AuditSeverity, CreateExpenseEntryParams, CreateTimeEntryParams, InvoiceStatus,
    MatterMemberRole, UpdateExpenseEntryParams, UpdateTimeEntryParams,
};

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route(
            "/api/matters/{id}/time",
            get(matter_time_list_handler).post(matter_time_create_handler),
        )
        .route(
            "/api/matters/{id}/time/{entry_id}",
            axum::routing::patch(matter_time_patch_handler).delete(matter_time_delete_handler),
        )
        .route(
            "/api/matters/{id}/expenses",
            get(matter_expenses_list_handler).post(matter_expenses_create_handler),
        )
        .route(
            "/api/matters/{id}/expenses/{entry_id}",
            axum::routing::patch(matter_expenses_patch_handler)
                .delete(matter_expenses_delete_handler),
        )
        .route(
            "/api/matters/{id}/time-summary",
            get(matter_time_summary_handler),
        )
        .route(
            "/api/matters/{id}/invoices",
            get(matter_invoices_list_handler),
        )
        .route("/api/invoices/draft", post(invoices_draft_handler))
        .route("/api/invoices", post(invoices_save_handler))
        .route("/api/invoices/{id}", get(invoices_get_handler))
        .route(
            "/api/invoices/{id}/finalize",
            post(invoices_finalize_handler),
        )
        .route("/api/invoices/{id}/void", post(invoices_void_handler))
        .route("/api/invoices/{id}/payment", post(invoices_payment_handler))
        .route(
            "/api/matters/{id}/trust/deposit",
            post(matter_trust_deposit_handler),
        )
        .route(
            "/api/matters/{id}/trust/ledger",
            get(matter_trust_ledger_handler),
        )
}

pub(crate) async fn matter_time_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterTimeEntriesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entries = store
        .list_time_entries(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::time_entry_record_to_info)
        .collect();
    Ok(Json(MatterTimeEntriesResponse { entries }))
}

pub(crate) async fn matter_time_create_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<CreateTimeEntryRequest>,
) -> Result<(StatusCode, Json<TimeEntryInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;

    let timekeeper =
        crate::channels::web::server::parse_required_matter_field("timekeeper", &req.timekeeper)?;
    let description =
        crate::channels::web::server::parse_required_matter_field("description", &req.description)?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "timekeeper",
        &Some(timekeeper.clone()),
    )?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "description",
        &Some(description.clone()),
    )?;
    let hours = crate::channels::web::server::parse_decimal_field("hours", &req.hours)?;
    let hourly_rate =
        crate::channels::web::server::parse_optional_decimal_field("hourly_rate", req.hourly_rate)?;
    let entry_date = crate::channels::web::server::parse_date_only("entry_date", &req.entry_date)?;
    let billable = req.billable.unwrap_or(true);

    let created = store
        .create_time_entry(
            &state.user_id,
            &matter_id,
            &CreateTimeEntryParams {
                timekeeper,
                description,
                hours,
                hourly_rate,
                entry_date,
                billable,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::time_entry_record_to_info(
            created,
        )),
    ))
}

pub(crate) async fn matter_time_patch_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, entry_id)): Path<(String, String)>,
    Json(req): Json<UpdateTimeEntryRequest>,
) -> Result<Json<TimeEntryInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entry_id = crate::channels::web::server::parse_uuid(&entry_id, "entry_id")?;

    let timekeeper = req.timekeeper.map(|value| value.trim().to_string());
    if let Some(ref value) = timekeeper {
        crate::channels::web::server::validate_optional_matter_field_length(
            "timekeeper",
            &Some(value.clone()),
        )?;
    }
    let description = req.description.map(|value| value.trim().to_string());
    if let Some(ref value) = description {
        crate::channels::web::server::validate_optional_matter_field_length(
            "description",
            &Some(value.clone()),
        )?;
    }
    let hours = req
        .hours
        .as_deref()
        .map(|value| crate::channels::web::server::parse_decimal_field("hours", value))
        .transpose()?;
    let hourly_rate = match req.hourly_rate {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(Some(crate::channels::web::server::parse_decimal_field(
            "hourly_rate",
            &raw,
        )?)),
    };
    let entry_date = req
        .entry_date
        .as_deref()
        .map(|value| crate::channels::web::server::parse_date_only("entry_date", value))
        .transpose()?;

    let updated = store
        .update_time_entry(
            &state.user_id,
            &matter_id,
            entry_id,
            &UpdateTimeEntryParams {
                timekeeper,
                description,
                hours,
                hourly_rate,
                entry_date,
                billable: req.billable,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Time entry not found".to_string()))?;

    Ok(Json(
        crate::channels::web::server::time_entry_record_to_info(updated),
    ))
}

pub(crate) async fn matter_time_delete_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entry_id = crate::channels::web::server::parse_uuid(&entry_id, "entry_id")?;

    let existing = store
        .get_time_entry(&state.user_id, &matter_id, entry_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Time entry not found".to_string()))?;
    if existing.billed_invoice_id.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Time entry is billed and cannot be deleted".to_string(),
        ));
    }

    let deleted = store
        .delete_time_entry(&state.user_id, &matter_id, entry_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Time entry not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_expenses_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterExpenseEntriesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entries = store
        .list_expense_entries(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::expense_entry_record_to_info)
        .collect();
    Ok(Json(MatterExpenseEntriesResponse { entries }))
}

pub(crate) async fn matter_expenses_create_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<CreateExpenseEntryRequest>,
) -> Result<(StatusCode, Json<ExpenseEntryInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;

    let submitted_by = crate::channels::web::server::parse_required_matter_field(
        "submitted_by",
        &req.submitted_by,
    )?;
    let description =
        crate::channels::web::server::parse_required_matter_field("description", &req.description)?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "submitted_by",
        &Some(submitted_by.clone()),
    )?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "description",
        &Some(description.clone()),
    )?;
    let amount = crate::channels::web::server::parse_decimal_field("amount", &req.amount)?;
    let category = crate::channels::web::server::parse_expense_category(&req.category)?;
    let entry_date = crate::channels::web::server::parse_date_only("entry_date", &req.entry_date)?;
    let receipt_path = crate::channels::web::server::parse_optional_matter_field(req.receipt_path);
    crate::channels::web::server::validate_optional_matter_field_length(
        "receipt_path",
        &receipt_path,
    )?;
    let billable = req.billable.unwrap_or(true);

    let created = store
        .create_expense_entry(
            &state.user_id,
            &matter_id,
            &CreateExpenseEntryParams {
                submitted_by,
                description,
                amount,
                category,
                entry_date,
                receipt_path,
                billable,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::expense_entry_record_to_info(
            created,
        )),
    ))
}

pub(crate) async fn matter_expenses_patch_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, entry_id)): Path<(String, String)>,
    Json(req): Json<UpdateExpenseEntryRequest>,
) -> Result<Json<ExpenseEntryInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entry_id = crate::channels::web::server::parse_uuid(&entry_id, "entry_id")?;

    let submitted_by = req.submitted_by.map(|value| value.trim().to_string());
    if let Some(ref value) = submitted_by {
        crate::channels::web::server::validate_optional_matter_field_length(
            "submitted_by",
            &Some(value.clone()),
        )?;
    }
    let description = req.description.map(|value| value.trim().to_string());
    if let Some(ref value) = description {
        crate::channels::web::server::validate_optional_matter_field_length(
            "description",
            &Some(value.clone()),
        )?;
    }
    let amount = req
        .amount
        .as_deref()
        .map(|value| crate::channels::web::server::parse_decimal_field("amount", value))
        .transpose()?;
    let category = req
        .category
        .as_deref()
        .map(crate::channels::web::server::parse_expense_category)
        .transpose()?;
    let entry_date = req
        .entry_date
        .as_deref()
        .map(|value| crate::channels::web::server::parse_date_only("entry_date", value))
        .transpose()?;
    let receipt_path = req.receipt_path.map(|value| {
        value.and_then(|inner| {
            let trimmed = inner.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    });
    if let Some(Some(ref value)) = receipt_path {
        crate::channels::web::server::validate_optional_matter_field_length(
            "receipt_path",
            &Some(value.clone()),
        )?;
    }

    let updated = store
        .update_expense_entry(
            &state.user_id,
            &matter_id,
            entry_id,
            &UpdateExpenseEntryParams {
                submitted_by,
                description,
                amount,
                category,
                entry_date,
                receipt_path,
                billable: req.billable,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Expense entry not found".to_string()))?;

    Ok(Json(
        crate::channels::web::server::expense_entry_record_to_info(updated),
    ))
}

pub(crate) async fn matter_expenses_delete_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entry_id = crate::channels::web::server::parse_uuid(&entry_id, "entry_id")?;

    let existing = store
        .get_expense_entry(&state.user_id, &matter_id, entry_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Expense entry not found".to_string()))?;
    if existing.billed_invoice_id.is_some() {
        return Err((
            StatusCode::CONFLICT,
            "Expense entry is billed and cannot be deleted".to_string(),
        ));
    }

    let deleted = store
        .delete_expense_entry(&state.user_id, &matter_id, entry_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Expense entry not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_time_summary_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterTimeSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let summary = store
        .matter_time_summary(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(
        crate::channels::web::server::matter_time_summary_to_response(summary),
    ))
}

pub(crate) async fn matter_invoices_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Query(query): Query<MatterInvoicesQuery>,
) -> Result<Json<MatterInvoicesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;

    let limit = query
        .limit
        .unwrap_or(crate::channels::web::server::MATTER_INVOICES_DEFAULT_LIMIT);
    if limit == 0 || limit > crate::channels::web::server::MATTER_INVOICES_MAX_LIMIT {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'limit' must be between 1 and {}",
                crate::channels::web::server::MATTER_INVOICES_MAX_LIMIT
            ),
        ));
    }

    let mut invoices = store
        .list_invoices(&state.user_id, Some(&matter_id))
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    invoices.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });

    Ok(Json(MatterInvoicesResponse {
        matter_id,
        invoices: invoices
            .into_iter()
            .take(limit)
            .map(crate::channels::web::server::invoice_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn invoices_draft_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Json(req): Json<DraftInvoiceRequest>,
) -> Result<Json<InvoiceDraftResponse>, (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&req.matter_id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let invoice_number = crate::channels::web::server::parse_required_matter_field(
        "invoice_number",
        &req.invoice_number,
    )?;
    let due_date = req
        .due_date
        .as_deref()
        .map(|raw| crate::channels::web::server::parse_date_only("due_date", raw))
        .transpose()?;
    let notes = crate::channels::web::server::parse_optional_matter_field(req.notes);
    let draft = crate::legal::billing::draft_invoice(
        store.as_ref(),
        &state.user_id,
        &matter_id,
        &invoice_number,
        due_date,
        notes,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok(Json(InvoiceDraftResponse {
        invoice: crate::channels::web::server::invoice_draft_to_info(&draft.invoice),
        line_items: draft
            .line_items
            .iter()
            .map(crate::channels::web::server::invoice_line_item_params_to_info)
            .collect(),
    }))
}

pub(crate) async fn invoices_save_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Json(req): Json<DraftInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceDetailResponse>), (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&req.matter_id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let invoice_number = crate::channels::web::server::parse_required_matter_field(
        "invoice_number",
        &req.invoice_number,
    )?;
    let due_date = req
        .due_date
        .as_deref()
        .map(|raw| crate::channels::web::server::parse_date_only("due_date", raw))
        .transpose()?;
    let notes = crate::channels::web::server::parse_optional_matter_field(req.notes);
    let draft = crate::legal::billing::draft_invoice(
        store.as_ref(),
        &state.user_id,
        &matter_id,
        &invoice_number,
        due_date,
        notes,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let (invoice, line_items) =
        crate::legal::billing::save_draft(store.as_ref(), &state.user_id, &draft)
            .await
            .map_err(|err| {
                let message = err.to_string();
                if message.contains("UNIQUE constraint")
                    || message.contains("duplicate key value")
                    || message.contains("invoice_number")
                {
                    (
                        StatusCode::CONFLICT,
                        "Invoice number already exists".to_string(),
                    )
                } else {
                    (StatusCode::INTERNAL_SERVER_ERROR, message)
                }
            })?;
    Ok((
        StatusCode::CREATED,
        Json(InvoiceDetailResponse {
            invoice: crate::channels::web::server::invoice_record_to_info(invoice),
            line_items: line_items
                .into_iter()
                .map(crate::channels::web::server::invoice_line_item_record_to_info)
                .collect(),
        }),
    ))
}

pub(crate) async fn invoices_get_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let invoice_id = crate::channels::web::server::parse_uuid(&id, "invoice_id")?;
    // Fetch the invoice first so we can derive its matter_id for the RBAC check.
    let invoice = store
        .get_invoice(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invoice not found".to_string()))?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &invoice.matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    let line_items = store
        .list_invoice_line_items(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(InvoiceDetailResponse {
        invoice: crate::channels::web::server::invoice_record_to_info(invoice),
        line_items: line_items
            .into_iter()
            .map(crate::channels::web::server::invoice_line_item_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn invoices_finalize_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let invoice_id = crate::channels::web::server::parse_uuid(&id, "invoice_id")?;
    // Pre-fetch the invoice to derive its matter_id for the RBAC check before mutating state.
    let preview = store
        .get_invoice(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invoice not found".to_string()))?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &preview.matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    let invoice =
        crate::legal::billing::finalize_invoice(store.as_ref(), &state.user_id, invoice_id)
            .await
            .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let line_items = store
        .list_invoice_line_items(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "invoice_finalized",
        state.user_id.as_str(),
        Some(invoice.matter_id.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "invoice_id": invoice.id.to_string(),
            "invoice_number": invoice.invoice_number.clone(),
            "matter_id": invoice.matter_id.clone(),
        }),
    )
    .await;
    Ok(Json(InvoiceDetailResponse {
        invoice: crate::channels::web::server::invoice_record_to_info(invoice),
        line_items: line_items
            .into_iter()
            .map(crate::channels::web::server::invoice_line_item_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn invoices_void_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let invoice_id = crate::channels::web::server::parse_uuid(&id, "invoice_id")?;
    let existing = store
        .get_invoice(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invoice not found".to_string()))?;
    // Voiding is irreversible — require Owner access on the matter.
    require_matter_access(
        &state.store,
        &state.user_id,
        &existing.matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    if !matches!(existing.status, InvoiceStatus::Draft | InvoiceStatus::Sent) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "Cannot void invoice with status '{}'",
                existing.status.as_str()
            ),
        ));
    }

    let invoice = store
        .set_invoice_status(&state.user_id, invoice_id, InvoiceStatus::Void, None)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invoice not found".to_string()))?;
    let line_items = store
        .list_invoice_line_items(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(InvoiceDetailResponse {
        invoice: crate::channels::web::server::invoice_record_to_info(invoice),
        line_items: line_items
            .into_iter()
            .map(crate::channels::web::server::invoice_line_item_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn invoices_payment_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<RecordInvoicePaymentRequest>,
) -> Result<Json<RecordInvoicePaymentResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let invoice_id = crate::channels::web::server::parse_uuid(&id, "invoice_id")?;
    // Pre-fetch the invoice to derive its matter_id for the RBAC check before mutating state.
    let invoice_preview = store
        .get_invoice(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Invoice not found".to_string()))?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &invoice_preview.matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|sc| (sc, "Insufficient permissions".to_string()))?;
    let amount = crate::channels::web::server::parse_decimal_field("amount", &req.amount)?;
    let recorded_by =
        crate::channels::web::server::parse_required_matter_field("recorded_by", &req.recorded_by)?;
    let payment_result = crate::legal::billing::record_payment(
        store.as_ref(),
        &state.user_id,
        invoice_id,
        amount,
        &recorded_by,
        req.draw_from_trust,
        req.description.as_deref(),
    )
    .await;
    let (invoice, trust_entry) = match payment_result {
        Ok(result) => result,
        Err(err) => {
            if req.draw_from_trust && err.to_ascii_lowercase().contains("insufficient") {
                crate::channels::web::server::record_legal_audit_event(
                    state.as_ref(),
                    "trust_withdrawal_rejected",
                    state.user_id.as_str(),
                    None,
                    AuditSeverity::Warn,
                    serde_json::json!({
                        "invoice_id": invoice_id.to_string(),
                        "amount": amount.to_string(),
                        "reason": "insufficient_balance",
                    }),
                )
                .await;
            }
            return Err((StatusCode::BAD_REQUEST, err));
        }
    };
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "payment_recorded",
        state.user_id.as_str(),
        Some(invoice.matter_id.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "invoice_id": invoice.id.to_string(),
            "matter_id": invoice.matter_id.clone(),
            "amount": amount.to_string(),
            "draw_from_trust": req.draw_from_trust,
            "trust_entry_created": trust_entry.is_some(),
        }),
    )
    .await;
    Ok(Json(RecordInvoicePaymentResponse {
        invoice: crate::channels::web::server::invoice_record_to_info(invoice),
        trust_entry: trust_entry
            .map(crate::channels::web::server::trust_ledger_entry_record_to_info),
    }))
}

pub(crate) async fn matter_trust_deposit_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<TrustDepositRequest>,
) -> Result<(StatusCode, Json<TrustLedgerEntryInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let amount = crate::channels::web::server::parse_decimal_field("amount", &req.amount)?;
    let recorded_by =
        crate::channels::web::server::parse_required_matter_field("recorded_by", &req.recorded_by)?;
    let description = crate::channels::web::server::parse_optional_matter_field(req.description)
        .unwrap_or_else(|| "Trust deposit".to_string());
    let entry = crate::legal::billing::record_trust_deposit(
        store.as_ref(),
        &state.user_id,
        &matter_id,
        amount,
        &recorded_by,
        &description,
    )
    .await
    .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "trust_deposit",
        state.user_id.as_str(),
        Some(matter_id.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "matter_id": matter_id.clone(),
            "entry_id": entry.id.to_string(),
            "amount": entry.amount.to_string(),
            "recorded_by": recorded_by,
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::trust_ledger_entry_record_to_info(entry)),
    ))
}

pub(crate) async fn matter_trust_ledger_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<TrustLedgerResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let entries = store
        .list_trust_ledger_entries(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let balance = store
        .current_trust_balance(&state.user_id, &matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(TrustLedgerResponse {
        matter_id,
        balance: balance.to_string(),
        entries: entries
            .into_iter()
            .map(crate::channels::web::server::trust_ledger_entry_record_to_info)
            .collect(),
    }))
}
