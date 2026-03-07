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
    AuditSeverity, CreateBillingRateScheduleParams, CreateExpenseEntryParams,
    CreateTimeEntryParams, InvoiceStatus, MatterMemberRole, UpdateBillingRateScheduleParams,
    UpdateExpenseEntryParams, UpdateTimeEntryParams, UpsertTrustAccountParams, UserRole,
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
        .route(
            "/api/trust/account",
            get(trust_account_get_handler).put(trust_account_put_handler),
        )
        .route(
            "/api/trust/statements/import",
            post(trust_statements_import_handler),
        )
        .route(
            "/api/trust/reconciliations/compute",
            post(trust_reconciliations_compute_handler),
        )
        .route(
            "/api/trust/reconciliations/{id}/signoff",
            post(trust_reconciliations_signoff_handler),
        )
        .route(
            "/api/billing/rates",
            get(billing_rates_list_handler).post(billing_rates_create_handler),
        )
        .route(
            "/api/billing/rates/{id}",
            axum::routing::patch(billing_rates_patch_handler),
        )
        .route("/api/invoices/{id}/ledes", get(invoice_ledes_handler))
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
    let task_code = crate::legal::billing::normalize_task_code(req.task_code)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let activity_code = crate::legal::billing::normalize_activity_code(req.activity_code)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let (resolved_rate, rate_source) = crate::legal::billing::resolve_time_entry_rate(
        store.as_ref(),
        &state.user_id,
        &matter_id,
        &timekeeper,
        entry_date,
        hourly_rate,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let auto_block_reason = crate::legal::billing::detect_block_billing(&description);
    let block_billing_flag = req
        .block_billing_flag
        .unwrap_or_else(|| auto_block_reason.is_some());
    let block_billing_reason = if !block_billing_flag {
        None
    } else {
        crate::channels::web::server::parse_optional_matter_field(req.block_billing_reason)
            .or(auto_block_reason)
    };

    let created = store
        .create_time_entry(
            &state.user_id,
            &matter_id,
            &CreateTimeEntryParams {
                timekeeper,
                description,
                hours,
                hourly_rate,
                task_code,
                activity_code,
                resolved_rate,
                rate_source,
                entry_date,
                billable,
                block_billing_flag,
                block_billing_reason,
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
    let existing = store
        .get_time_entry(&state.user_id, &matter_id, entry_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Time entry not found".to_string()))?;

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
    let task_code = match req.task_code {
        None => None,
        Some(value) => Some(
            crate::legal::billing::normalize_task_code(value)
                .map_err(|err| (StatusCode::BAD_REQUEST, err))?,
        ),
    };
    let activity_code = match req.activity_code {
        None => None,
        Some(value) => Some(
            crate::legal::billing::normalize_activity_code(value)
                .map_err(|err| (StatusCode::BAD_REQUEST, err))?,
        ),
    };
    let merged_timekeeper = timekeeper
        .clone()
        .unwrap_or_else(|| existing.timekeeper.clone());
    let merged_entry_date = entry_date.unwrap_or(existing.entry_date);
    let merged_hourly_rate = hourly_rate.unwrap_or(existing.hourly_rate);
    let merged_description = description
        .clone()
        .unwrap_or_else(|| existing.description.clone());
    let (resolved_rate, rate_source) = crate::legal::billing::resolve_time_entry_rate(
        store.as_ref(),
        &state.user_id,
        &matter_id,
        &merged_timekeeper,
        merged_entry_date,
        merged_hourly_rate,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let auto_block_reason = crate::legal::billing::detect_block_billing(&merged_description);
    let requested_block_reason =
        crate::channels::web::server::parse_optional_matter_field_patch(req.block_billing_reason);
    let block_billing_flag = req
        .block_billing_flag
        .or_else(|| {
            if requested_block_reason.is_some() {
                Some(true)
            } else {
                None
            }
        })
        .unwrap_or(existing.block_billing_flag || auto_block_reason.is_some());
    let block_billing_reason = if !block_billing_flag {
        Some(None)
    } else {
        Some(
            requested_block_reason
                .unwrap_or(existing.block_billing_reason)
                .or(auto_block_reason),
        )
    };

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
                task_code,
                activity_code,
                resolved_rate: Some(resolved_rate),
                rate_source: Some(rate_source),
                entry_date,
                billable: req.billable,
                block_billing_flag: Some(block_billing_flag),
                block_billing_reason,
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
        crate::channels::web::server::parse_optional_matter_field(req.reference_number),
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
    let account = match store.get_primary_trust_account(&state.user_id).await {
        Ok(Some(account)) => {
            let account_balance = store
                .current_trust_account_balance(&state.user_id, account.id)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
            Some((account, account_balance))
        }
        Ok(None) => None,
        Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    };
    let latest_reconciliation = if let Some((account_record, _)) = account.as_ref() {
        store
            .latest_trust_reconciliation_for_account(&state.user_id, account_record.id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
            .map(|record| {
                crate::channels::web::server::trust_reconciliation_record_to_info(record, None)
            })
    } else {
        None
    };
    Ok(Json(TrustLedgerResponse {
        matter_id,
        balance: balance.to_string(),
        account: account.as_ref().map(|(record, balance)| {
            crate::channels::web::server::trust_account_record_to_info(
                record.clone(),
                Some(*balance),
            )
        }),
        account_balance: account.as_ref().map(|(_, balance)| balance.to_string()),
        latest_reconciliation,
        entries: entries
            .into_iter()
            .map(crate::channels::web::server::trust_ledger_entry_record_to_info)
            .collect(),
    }))
}

async fn trust_reconciliation_info_for_response(
    state: &GatewayState,
    record: crate::db::TrustReconciliationRecord,
    include_report: bool,
) -> Result<TrustReconciliationInfo, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let statement = store
        .get_trust_statement_import(&state.user_id, record.statement_import_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Trust statement import not found".to_string(),
        ))?;
    let report_markdown = if include_report {
        let account = store
            .get_primary_trust_account(&state.user_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let lines = store
            .list_trust_statement_lines(&state.user_id, statement.id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let ledger = store
            .list_trust_ledger_entries_for_account(&state.user_id, record.trust_account_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        account.map(|account| {
            crate::legal::trust::render_examiner_report(
                &account, &statement, &lines, &record, &ledger,
            )
        })
    } else {
        None
    };

    Ok(crate::channels::web::server::trust_reconciliation_record_to_info(record, report_markdown))
}

pub(crate) async fn trust_account_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<TrustAccountInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let account = store
        .get_primary_trust_account(&state.user_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Primary trust account not configured".to_string(),
        ))?;
    let balance = store
        .current_trust_account_balance(&state.user_id, account.id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(
        crate::channels::web::server::trust_account_record_to_info(account, Some(balance)),
    ))
}

pub(crate) async fn trust_account_put_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Json(req): Json<UpdateTrustAccountRequest>,
) -> Result<Json<TrustAccountInfo>, (StatusCode, String)> {
    // Trust account is a firm-wide resource; only Admins may modify it.
    if state.store.is_some() && principal.role != UserRole::Admin {
        return Err((
            StatusCode::FORBIDDEN,
            "Only administrators can update trust account settings".to_string(),
        ));
    }
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let name = crate::channels::web::server::parse_required_matter_field("name", &req.name)?;
    let bank_name = crate::channels::web::server::parse_optional_matter_field(req.bank_name);
    let account_number_last4 =
        crate::channels::web::server::parse_optional_matter_field(req.account_number_last4);
    let account = store
        .upsert_primary_trust_account(
            &state.user_id,
            &UpsertTrustAccountParams {
                name,
                bank_name,
                account_number_last4,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let balance = store
        .current_trust_account_balance(&state.user_id, account.id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(
        crate::channels::web::server::trust_account_record_to_info(account, Some(balance)),
    ))
}

pub(crate) async fn trust_statements_import_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ImportTrustStatementRequest>,
) -> Result<(StatusCode, Json<ImportTrustStatementResponse>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let account = store
        .get_primary_trust_account(&state.user_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::CONFLICT,
            "Primary trust account must be configured before importing statements".to_string(),
        ))?;
    let imported_by =
        crate::channels::web::server::parse_required_matter_field("imported_by", &req.imported_by)?;
    let statement_date = req
        .statement_date
        .as_deref()
        .map(|value| crate::channels::web::server::parse_date_only("statement_date", value))
        .transpose()?;
    let parsed = crate::legal::trust::parse_statement_csv(&req.csv, statement_date)
        .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    let (statement, lines) = store
        .import_trust_statement(
            &state.user_id,
            &crate::db::CreateTrustStatementImportParams {
                trust_account_id: account.id,
                statement_date: parsed.statement_date,
                starting_balance: parsed.starting_balance,
                ending_balance: parsed.ending_balance,
                imported_by: imported_by.clone(),
            },
            &parsed
                .lines
                .iter()
                .map(|line| crate::db::CreateTrustStatementLineParams {
                    entry_date: line.entry_date,
                    description: line.description.clone(),
                    debit: line.debit,
                    credit: line.credit,
                    running_balance: line.running_balance,
                    reference_number: line.reference_number.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "trust_statement_imported",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "trust_account_id": account.id.to_string(),
            "statement_import_id": statement.id.to_string(),
            "row_count": statement.row_count,
            "imported_by": imported_by,
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(ImportTrustStatementResponse {
            statement: crate::channels::web::server::trust_statement_import_record_to_info(
                statement,
            ),
            lines: lines
                .into_iter()
                .map(crate::channels::web::server::trust_statement_line_record_to_info)
                .collect(),
        }),
    ))
}

pub(crate) async fn trust_reconciliations_compute_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ComputeTrustReconciliationRequest>,
) -> Result<(StatusCode, Json<TrustReconciliationInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let statement_import_id =
        crate::channels::web::server::parse_uuid(&req.statement_import_id, "statement_import_id")?;
    let statement = store
        .get_trust_statement_import(&state.user_id, statement_import_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Trust statement import not found".to_string(),
        ))?;
    let reconciliation = store
        .compute_trust_reconciliation(
            &state.user_id,
            &crate::db::ComputeTrustReconciliationParams {
                trust_account_id: statement.trust_account_id,
                statement_import_id,
            },
        )
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "trust_reconciliation_computed",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "statement_import_id": statement_import_id.to_string(),
            "trust_account_id": statement.trust_account_id.to_string(),
            "reconciliation_id": reconciliation.id.to_string(),
            "difference": reconciliation.difference.to_string(),
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(trust_reconciliation_info_for_response(state.as_ref(), reconciliation, true).await?),
    ))
}

pub(crate) async fn trust_reconciliations_signoff_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<SignoffTrustReconciliationRequest>,
) -> Result<Json<TrustReconciliationInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let reconciliation_id = crate::channels::web::server::parse_uuid(&id, "reconciliation_id")?;
    let signed_off_by = crate::channels::web::server::parse_required_matter_field(
        "signed_off_by",
        &req.signed_off_by,
    )?;
    let reconciliation = store
        .signoff_trust_reconciliation(&state.user_id, reconciliation_id, &signed_off_by)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Trust reconciliation not found".to_string(),
        ))?;
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "trust_reconciliation_signed_off",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "reconciliation_id": reconciliation.id.to_string(),
            "signed_off_by": signed_off_by,
        }),
    )
    .await;
    Ok(Json(
        trust_reconciliation_info_for_response(state.as_ref(), reconciliation, true).await?,
    ))
}

pub(crate) async fn billing_rates_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<BillingRateSchedulesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let schedules = store
        .list_billing_rate_schedules(&state.user_id, None, None)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(BillingRateSchedulesResponse {
        schedules: schedules
            .into_iter()
            .map(crate::channels::web::server::billing_rate_schedule_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn billing_rates_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateBillingRateScheduleRequest>,
) -> Result<(StatusCode, Json<BillingRateScheduleInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = req
        .matter_id
        .map(|value| crate::channels::web::server::sanitize_matter_id_for_route(&value))
        .transpose()?;
    if let Some(ref matter_id) = matter_id {
        crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), matter_id).await?;
    }
    let timekeeper =
        crate::channels::web::server::parse_required_matter_field("timekeeper", &req.timekeeper)?;
    let rate = crate::channels::web::server::parse_decimal_field("rate", &req.rate)?;
    let effective_start =
        crate::channels::web::server::parse_date_only("effective_start", &req.effective_start)?;
    let effective_end = req
        .effective_end
        .as_deref()
        .map(|value| crate::channels::web::server::parse_date_only("effective_end", value))
        .transpose()?;
    let schedule = store
        .create_billing_rate_schedule(
            &state.user_id,
            &CreateBillingRateScheduleParams {
                matter_id,
                timekeeper,
                rate,
                effective_start,
                effective_end,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::billing_rate_schedule_record_to_info(schedule)),
    ))
}

pub(crate) async fn billing_rates_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateBillingRateScheduleRequest>,
) -> Result<Json<BillingRateScheduleInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let schedule_id = crate::channels::web::server::parse_uuid(&id, "schedule_id")?;
    let matter_id = match req.matter_id {
        None => None,
        Some(None) => Some(None),
        Some(Some(value)) => Some(Some(
            crate::channels::web::server::sanitize_matter_id_for_route(&value)?,
        )),
    };
    if let Some(Some(ref matter_id)) = matter_id {
        crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), matter_id).await?;
    }
    let timekeeper = req
        .timekeeper
        .map(|value| {
            crate::channels::web::server::parse_required_matter_field("timekeeper", &value)
        })
        .transpose()?;
    let rate = req
        .rate
        .as_deref()
        .map(|value| crate::channels::web::server::parse_decimal_field("rate", value))
        .transpose()?;
    let effective_start = req
        .effective_start
        .as_deref()
        .map(|value| crate::channels::web::server::parse_date_only("effective_start", value))
        .transpose()?;
    let effective_end = match req.effective_end {
        None => None,
        Some(None) => Some(None),
        Some(Some(value)) => Some(Some(crate::channels::web::server::parse_date_only(
            "effective_end",
            &value,
        )?)),
    };
    let updated = store
        .update_billing_rate_schedule(
            &state.user_id,
            schedule_id,
            &UpdateBillingRateScheduleParams {
                matter_id,
                timekeeper,
                rate,
                effective_start,
                effective_end,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Billing rate schedule not found".to_string(),
        ))?;
    Ok(Json(
        crate::channels::web::server::billing_rate_schedule_record_to_info(updated),
    ))
}

pub(crate) async fn invoice_ledes_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Query(query): Query<InvoiceLedesQuery>,
) -> Result<Json<InvoiceLedesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let invoice_id = crate::channels::web::server::parse_uuid(&id, "invoice_id")?;
    let format = query
        .format
        .unwrap_or_else(|| "98b".to_string())
        .to_ascii_lowercase();
    if format != "98b" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Only LEDES 98B export is supported in this phase".to_string(),
        ));
    }
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

    let matter = store
        .get_matter_db(&state.user_id, &invoice.matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;
    let line_items = store
        .list_invoice_line_items(&state.user_id, invoice_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let invoice_date = invoice
        .issued_date
        .unwrap_or_else(|| invoice.created_at.date_naive());
    let mut service_dates = Vec::new();
    let mut ledes_items = Vec::new();
    for (index, item) in line_items.iter().enumerate() {
        let mut line_item_date = invoice_date;
        if let Some(time_entry_id) = item.time_entry_id
            && let Some(entry) = store
                .get_time_entry(&state.user_id, &invoice.matter_id, time_entry_id)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        {
            line_item_date = entry.entry_date;
        }
        if let Some(expense_entry_id) = item.expense_entry_id
            && let Some(entry) = store
                .get_expense_entry(&state.user_id, &invoice.matter_id, expense_entry_id)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        {
            line_item_date = entry.entry_date;
        }
        service_dates.push(line_item_date);
        ledes_items.push(crate::legal::ledes::Ledes98BLineItem {
            line_item_number: index + 1,
            line_item_type: if item.expense_entry_id.is_some() {
                "EXP".to_string()
            } else {
                "FEE".to_string()
            },
            units: item.quantity,
            adjustment_amount: rust_decimal::Decimal::ZERO,
            total: item.amount,
            line_item_date,
            task_code: item.task_code.clone(),
            expense_code: None,
            activity_code: item.activity_code.clone(),
            timekeeper_id: item.timekeeper.clone(),
            description: item.description.clone(),
            unit_cost: item.unit_price,
            timekeeper_name: item.timekeeper.clone(),
            timekeeper_classification: Some("OT".to_string()),
        });
    }
    let billing_start_date = service_dates.iter().min().copied().unwrap_or(invoice_date);
    let billing_end_date = service_dates.iter().max().copied().unwrap_or(invoice_date);
    let client_matter_id = matter
        .custom_fields
        .get("client_matter_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(matter.matter_id.as_str())
        .to_string();
    let po_number = matter
        .custom_fields
        .get("po_number")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());
    let content = crate::legal::ledes::export_ledes98b(
        &crate::legal::ledes::Ledes98BInvoiceContext {
            invoice_date,
            invoice_number: invoice.invoice_number.clone(),
            client_id: matter.client_id.to_string(),
            law_firm_matter_id: matter.matter_id.clone(),
            invoice_total: invoice.total,
            billing_start_date,
            billing_end_date,
            law_firm_id: state.user_id.clone(),
            client_matter_id,
            po_number,
        },
        &ledes_items,
    );
    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "invoice_ledes_exported",
        state.user_id.as_str(),
        Some(invoice.matter_id.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "invoice_id": invoice_id.to_string(),
            "format": "98b",
            "line_item_count": ledes_items.len(),
        }),
    )
    .await;
    Ok(Json(InvoiceLedesResponse {
        invoice_id: invoice_id.to_string(),
        format,
        content,
    }))
}
