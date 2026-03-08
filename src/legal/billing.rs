use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::{
    BillingRateScheduleRecord, BillingRateSource, CreateInvoiceLineItemParams, CreateInvoiceParams,
    CreateTrustLedgerEntryParams, Database, InvoiceLineItemRecord, InvoiceRecord, InvoiceStatus,
    RecordInvoicePaymentParams, TrustLedgerEntryRecord, TrustLedgerEntryType,
};
use crate::error::DatabaseError;

static UTBMS_TASK_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^B\d{3}$").expect("valid UTBMS task regex"));
static UTBMS_ACTIVITY_CODE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^A\d{3}$").expect("valid UTBMS activity regex"));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingRateFallbackReason {
    ManualOverride,
    NoRateFound,
}

impl BillingRateFallbackReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualOverride => "manual_override",
            Self::NoRateFound => "no_rate_found",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedTimeEntryRate {
    pub rate: Option<Decimal>,
    pub source: Option<BillingRateSource>,
    pub matched_schedule: Option<BillingRateScheduleRecord>,
    pub fallback: Option<BillingRateFallbackReason>,
}

#[derive(Debug, Clone)]
pub struct DraftInvoiceLineItem {
    pub params: CreateInvoiceLineItemParams,
    pub rate_resolution: Option<ResolvedTimeEntryRate>,
}

#[derive(Debug, Clone, Copy)]
pub struct PersistedTimeEntryRateSnapshot {
    pub entry_date: NaiveDate,
    pub manual_hourly_rate: Option<Decimal>,
    pub persisted_rate: Option<Decimal>,
    pub persisted_source: Option<BillingRateSource>,
}

#[derive(Debug, Clone)]
pub struct DraftInvoiceResult {
    pub invoice: CreateInvoiceParams,
    pub line_items: Vec<DraftInvoiceLineItem>,
}

#[derive(Debug)]
pub enum BillingRateScheduleValidationError {
    InvalidRange,
    Overlap {
        existing: Box<BillingRateScheduleRecord>,
    },
    Database(DatabaseError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedesRequiredField {
    TaskCode,
    ActivityCode,
}

impl LedesRequiredField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TaskCode => "task_code",
            Self::ActivityCode => "activity_code",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LedesExportValidationError {
    pub line_item_id: Uuid,
    pub time_entry_id: Option<Uuid>,
    pub description: String,
    pub sort_order: i32,
    pub missing_fields: Vec<LedesRequiredField>,
}

fn schedule_applies(schedule: &BillingRateScheduleRecord, entry_date: NaiveDate) -> bool {
    schedule.effective_start <= entry_date
        && schedule.effective_end.is_none_or(|end| end >= entry_date)
}

fn best_schedule<'a>(
    schedules: impl IntoIterator<Item = &'a BillingRateScheduleRecord>,
    entry_date: NaiveDate,
) -> Option<&'a BillingRateScheduleRecord> {
    schedules
        .into_iter()
        .filter(|schedule| schedule_applies(schedule, entry_date))
        .max_by_key(|schedule| schedule.effective_start)
}

fn schedule_matches_scope(schedule: &BillingRateScheduleRecord, matter_id: Option<&str>) -> bool {
    match matter_id {
        Some(matter_id) => schedule.matter_id.as_deref() == Some(matter_id),
        None => schedule.matter_id.is_none(),
    }
}

async fn load_scoped_schedules(
    db: &dyn Database,
    user_id: &str,
    matter_id: Option<&str>,
    timekeeper: &str,
) -> Result<Vec<BillingRateScheduleRecord>, DatabaseError> {
    Ok(db
        .list_billing_rate_schedules(user_id, matter_id, Some(timekeeper))
        .await?
        .into_iter()
        .filter(|schedule| schedule_matches_scope(schedule, matter_id))
        .collect())
}

fn date_ranges_overlap(
    left_start: NaiveDate,
    left_end: Option<NaiveDate>,
    right_start: NaiveDate,
    right_end: Option<NaiveDate>,
) -> bool {
    left_end.is_none_or(|end| end >= right_start) && right_end.is_none_or(|end| end >= left_start)
}

fn matching_schedule_for_snapshot(
    schedules: &[BillingRateScheduleRecord],
    entry_date: NaiveDate,
    resolved_rate: Option<Decimal>,
) -> Option<BillingRateScheduleRecord> {
    let same_rate = schedules
        .iter()
        .filter(|schedule| schedule_applies(schedule, entry_date))
        .filter(|schedule| resolved_rate.is_none_or(|rate| schedule.rate == rate))
        .max_by_key(|schedule| schedule.effective_start)
        .cloned();
    if same_rate.is_some() {
        same_rate
    } else {
        best_schedule(schedules.iter(), entry_date).cloned()
    }
}

pub fn normalize_task_code(raw: Option<String>) -> Result<Option<String>, String> {
    let value = raw
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty());
    if let Some(ref value) = value
        && !UTBMS_TASK_CODE_RE.is_match(value)
    {
        return Err("task_code must match UTBMS format like B110".to_string());
    }
    Ok(value)
}

pub fn normalize_activity_code(raw: Option<String>) -> Result<Option<String>, String> {
    let value = raw
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty());
    if let Some(ref value) = value
        && !UTBMS_ACTIVITY_CODE_RE.is_match(value)
    {
        return Err("activity_code must match UTBMS format like A101".to_string());
    }
    Ok(value)
}

pub fn detect_block_billing(description: &str) -> Option<String> {
    let normalized = description.trim().to_ascii_lowercase();
    let separator_count = [
        normalized.contains(';'),
        normalized.contains(" and "),
        normalized.contains(" / "),
        normalized.matches(',').count() >= 2,
    ]
    .into_iter()
    .filter(|matched| *matched)
    .count();

    if separator_count >= 2 || normalized.contains(';') {
        Some("entry appears to describe multiple tasks in one billed block".to_string())
    } else {
        None
    }
}

pub fn validate_billing_rate_schedule_date_range(
    effective_start: NaiveDate,
    effective_end: Option<NaiveDate>,
) -> Result<(), BillingRateScheduleValidationError> {
    if effective_end.is_some_and(|end| end < effective_start) {
        Err(BillingRateScheduleValidationError::InvalidRange)
    } else {
        Ok(())
    }
}

pub async fn ensure_no_overlapping_billing_rate_schedule(
    db: &dyn Database,
    user_id: &str,
    matter_id: Option<&str>,
    timekeeper: &str,
    effective_start: NaiveDate,
    effective_end: Option<NaiveDate>,
    excluding_schedule_id: Option<Uuid>,
) -> Result<(), BillingRateScheduleValidationError> {
    validate_billing_rate_schedule_date_range(effective_start, effective_end)?;
    let schedules = load_scoped_schedules(db, user_id, matter_id, timekeeper)
        .await
        .map_err(BillingRateScheduleValidationError::Database)?;
    for schedule in schedules {
        if excluding_schedule_id.is_some_and(|id| schedule.id == id) {
            continue;
        }
        if date_ranges_overlap(
            effective_start,
            effective_end,
            schedule.effective_start,
            schedule.effective_end,
        ) {
            return Err(BillingRateScheduleValidationError::Overlap {
                existing: Box::new(schedule),
            });
        }
    }
    Ok(())
}

pub async fn resolve_time_entry_rate_details(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    timekeeper: &str,
    entry_date: NaiveDate,
    manual_hourly_rate: Option<Decimal>,
) -> Result<ResolvedTimeEntryRate, DatabaseError> {
    let matter_schedules = load_scoped_schedules(db, user_id, Some(matter_id), timekeeper).await?;
    if let Some(schedule) = best_schedule(matter_schedules.iter(), entry_date).cloned() {
        return Ok(ResolvedTimeEntryRate {
            rate: Some(schedule.rate),
            source: Some(BillingRateSource::MatterOverride),
            matched_schedule: Some(schedule),
            fallback: None,
        });
    }

    let default_schedules = load_scoped_schedules(db, user_id, None, timekeeper).await?;
    if let Some(schedule) = best_schedule(default_schedules.iter(), entry_date).cloned() {
        return Ok(ResolvedTimeEntryRate {
            rate: Some(schedule.rate),
            source: Some(BillingRateSource::TimekeeperDefault),
            matched_schedule: Some(schedule),
            fallback: None,
        });
    }

    Ok(ResolvedTimeEntryRate {
        rate: manual_hourly_rate,
        source: manual_hourly_rate.map(|_| BillingRateSource::ManualOverride),
        matched_schedule: None,
        fallback: Some(if manual_hourly_rate.is_some() {
            BillingRateFallbackReason::ManualOverride
        } else {
            BillingRateFallbackReason::NoRateFound
        }),
    })
}

pub async fn review_time_entry_rate(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    timekeeper: &str,
    snapshot: PersistedTimeEntryRateSnapshot,
) -> Result<ResolvedTimeEntryRate, DatabaseError> {
    match snapshot.persisted_source {
        Some(BillingRateSource::MatterOverride) => {
            let schedules = load_scoped_schedules(db, user_id, Some(matter_id), timekeeper).await?;
            Ok(ResolvedTimeEntryRate {
                rate: snapshot.persisted_rate,
                source: Some(BillingRateSource::MatterOverride),
                matched_schedule: matching_schedule_for_snapshot(
                    &schedules,
                    snapshot.entry_date,
                    snapshot.persisted_rate,
                ),
                fallback: None,
            })
        }
        Some(BillingRateSource::TimekeeperDefault) => {
            let schedules = load_scoped_schedules(db, user_id, None, timekeeper).await?;
            Ok(ResolvedTimeEntryRate {
                rate: snapshot.persisted_rate,
                source: Some(BillingRateSource::TimekeeperDefault),
                matched_schedule: matching_schedule_for_snapshot(
                    &schedules,
                    snapshot.entry_date,
                    snapshot.persisted_rate,
                ),
                fallback: None,
            })
        }
        Some(BillingRateSource::ManualOverride) => Ok(ResolvedTimeEntryRate {
            rate: snapshot.persisted_rate.or(snapshot.manual_hourly_rate),
            source: Some(BillingRateSource::ManualOverride),
            matched_schedule: None,
            fallback: Some(BillingRateFallbackReason::ManualOverride),
        }),
        None if snapshot.persisted_rate.is_some() => Ok(ResolvedTimeEntryRate {
            rate: snapshot.persisted_rate,
            source: None,
            matched_schedule: None,
            fallback: None,
        }),
        None => {
            resolve_time_entry_rate_details(
                db,
                user_id,
                matter_id,
                timekeeper,
                snapshot.entry_date,
                snapshot.manual_hourly_rate,
            )
            .await
        }
    }
}

pub async fn resolve_time_entry_rate(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    timekeeper: &str,
    entry_date: NaiveDate,
    manual_hourly_rate: Option<Decimal>,
) -> Result<(Option<Decimal>, Option<BillingRateSource>), DatabaseError> {
    let resolution = resolve_time_entry_rate_details(
        db,
        user_id,
        matter_id,
        timekeeper,
        entry_date,
        manual_hourly_rate,
    )
    .await?;
    Ok((resolution.rate, resolution.source))
}

pub async fn draft_invoice(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    invoice_number: &str,
    due_date: Option<NaiveDate>,
    notes: Option<String>,
) -> Result<DraftInvoiceResult, DatabaseError> {
    let time_entries = db.list_time_entries(user_id, matter_id).await?;
    let expense_entries = db.list_expense_entries(user_id, matter_id).await?;

    let mut line_items = Vec::new();

    for entry in time_entries {
        if entry.billed_invoice_id.is_some() || !entry.billable {
            continue;
        }
        let rate_resolution = review_time_entry_rate(
            db,
            user_id,
            matter_id,
            &entry.timekeeper,
            PersistedTimeEntryRateSnapshot {
                entry_date: entry.entry_date,
                manual_hourly_rate: entry.hourly_rate,
                persisted_rate: entry.resolved_rate,
                persisted_source: entry.rate_source,
            },
        )
        .await?;
        let (resolved_rate, rate_source) = match (entry.resolved_rate, entry.rate_source) {
            (Some(rate), source) => (Some(rate), source),
            _ => (rate_resolution.rate, rate_resolution.source),
        };
        let unit_price = resolved_rate.unwrap_or(Decimal::ZERO);
        let amount = (entry.hours * unit_price).round_dp(2);
        line_items.push(DraftInvoiceLineItem {
            params: CreateInvoiceLineItemParams {
                description: format!(
                    "Time: {} ({} on {})",
                    entry.description,
                    entry.timekeeper,
                    entry.entry_date.format("%Y-%m-%d")
                ),
                quantity: entry.hours,
                unit_price,
                amount,
                time_entry_id: Some(entry.id),
                expense_entry_id: None,
                task_code: entry.task_code.clone(),
                activity_code: entry.activity_code.clone(),
                timekeeper: Some(entry.timekeeper.clone()),
                resolved_rate,
                rate_source,
                sort_order: i32::try_from(line_items.len()).unwrap_or(0),
            },
            rate_resolution: Some(rate_resolution),
        });
    }

    for entry in expense_entries {
        if entry.billed_invoice_id.is_some() || !entry.billable {
            continue;
        }
        line_items.push(DraftInvoiceLineItem {
            params: CreateInvoiceLineItemParams {
                description: format!(
                    "Expense: {} ({})",
                    entry.description,
                    entry.entry_date.format("%Y-%m-%d")
                ),
                quantity: Decimal::ONE,
                unit_price: entry.amount,
                amount: entry.amount.round_dp(2),
                time_entry_id: None,
                expense_entry_id: Some(entry.id),
                task_code: None,
                activity_code: None,
                timekeeper: None,
                resolved_rate: None,
                rate_source: None,
                sort_order: i32::try_from(line_items.len()).unwrap_or(0),
            },
            rate_resolution: None,
        });
    }

    let subtotal = line_items
        .iter()
        .fold(Decimal::ZERO, |acc, item| acc + item.params.amount)
        .round_dp(2);
    let tax = Decimal::ZERO;
    let total = (subtotal + tax).round_dp(2);

    Ok(DraftInvoiceResult {
        invoice: CreateInvoiceParams {
            matter_id: matter_id.to_string(),
            invoice_number: invoice_number.trim().to_string(),
            status: InvoiceStatus::Draft,
            issued_date: None,
            due_date,
            subtotal,
            tax,
            total,
            paid_amount: Decimal::ZERO,
            notes,
        },
        line_items,
    })
}

pub async fn save_draft(
    db: &dyn Database,
    user_id: &str,
    draft: &DraftInvoiceResult,
) -> Result<(InvoiceRecord, Vec<InvoiceLineItemRecord>), DatabaseError> {
    let line_item_params: Vec<_> = draft
        .line_items
        .iter()
        .map(|item| item.params.clone())
        .collect();
    validate_invoice_totals(&draft.invoice, &line_item_params)?;
    db.save_invoice_draft(user_id, &draft.invoice, &line_item_params)
        .await
}

pub async fn finalize_invoice(
    db: &dyn Database,
    user_id: &str,
    invoice_id: Uuid,
) -> Result<InvoiceRecord, String> {
    db.finalize_invoice_atomic(user_id, invoice_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Invoice not found".to_string())
}

pub async fn record_payment(
    db: &dyn Database,
    user_id: &str,
    invoice_id: Uuid,
    amount: Decimal,
    recorded_by: &str,
    draw_from_trust: bool,
    description: Option<&str>,
) -> Result<(InvoiceRecord, Option<TrustLedgerEntryRecord>), String> {
    if amount <= Decimal::ZERO {
        return Err("Payment amount must be greater than 0".to_string());
    }
    let result = db
        .record_invoice_payment(
            user_id,
            invoice_id,
            &RecordInvoicePaymentParams {
                amount,
                draw_from_trust,
                recorded_by: recorded_by.trim().to_string(),
                description: description.map(|value| value.trim().to_string()),
            },
        )
        .await
        .map_err(|e| match e {
            DatabaseError::Constraint(message)
                if message
                    .to_ascii_lowercase()
                    .contains("insufficient trust balance") =>
            {
                "Trust balance is insufficient for this payment".to_string()
            }
            DatabaseError::Constraint(message) => message,
            other => other.to_string(),
        })?
        .ok_or_else(|| "Invoice not found".to_string())?;
    Ok((result.invoice, result.trust_entry))
}

pub async fn record_trust_deposit(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    amount: Decimal,
    recorded_by: &str,
    description: &str,
    reference_number: Option<String>,
) -> Result<TrustLedgerEntryRecord, String> {
    if amount <= Decimal::ZERO {
        return Err("Deposit amount must be greater than 0".to_string());
    }
    db.append_trust_ledger_entry(
        user_id,
        matter_id,
        &CreateTrustLedgerEntryParams {
            trust_account_id: None,
            entry_type: TrustLedgerEntryType::Deposit,
            amount,
            delta: amount,
            description: description.trim().to_string(),
            reference_number,
            source: crate::db::TrustLedgerSource::Manual,
            invoice_id: None,
            recorded_by: recorded_by.trim().to_string(),
        },
    )
    .await
    .map_err(|e| e.to_string())
}

pub fn validate_ledes98b_line_items(
    line_items: &[InvoiceLineItemRecord],
) -> Vec<LedesExportValidationError> {
    let mut errors = Vec::new();

    for item in line_items {
        if item.expense_entry_id.is_some() {
            continue;
        }

        let mut missing_fields = Vec::new();
        if item
            .task_code
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            missing_fields.push(LedesRequiredField::TaskCode);
        }
        if item
            .activity_code
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
        {
            missing_fields.push(LedesRequiredField::ActivityCode);
        }
        if !missing_fields.is_empty() {
            errors.push(LedesExportValidationError {
                line_item_id: item.id,
                time_entry_id: item.time_entry_id,
                description: item.description.clone(),
                sort_order: item.sort_order,
                missing_fields,
            });
        }
    }

    errors.sort_by_key(|error| error.sort_order);
    errors
}

fn validate_invoice_totals(
    invoice: &CreateInvoiceParams,
    line_items: &[CreateInvoiceLineItemParams],
) -> Result<(), DatabaseError> {
    let computed_subtotal = line_items
        .iter()
        .fold(Decimal::ZERO, |acc, item| acc + item.amount)
        .round_dp(2);
    if computed_subtotal != invoice.subtotal.round_dp(2) {
        return Err(DatabaseError::Constraint(format!(
            "invoice subtotal {} does not match line-item sum {}",
            invoice.subtotal.round_dp(2),
            computed_subtotal
        )));
    }
    let computed_total = (invoice.subtotal + invoice.tax).round_dp(2);
    if computed_total != invoice.total.round_dp(2) {
        return Err(DatabaseError::Constraint(format!(
            "invoice total {} does not equal subtotal + tax {}",
            invoice.total.round_dp(2),
            computed_total
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn utbms_codes_are_normalized() {
        assert_eq!(
            normalize_task_code(Some("b110".to_string())).unwrap(),
            Some("B110".to_string())
        );
        assert_eq!(
            normalize_activity_code(Some("a101".to_string())).unwrap(),
            Some("A101".to_string())
        );
        assert!(normalize_task_code(Some("x110".to_string())).is_err());
    }

    #[test]
    fn block_billing_detector_flags_multi_task_entries() {
        assert!(detect_block_billing("Draft motion; revise affidavit").is_some());
        assert!(detect_block_billing("Research issue and call client / revise draft").is_some());
        assert!(detect_block_billing("Draft motion").is_none());
    }

    #[test]
    fn billing_rate_date_ranges_treat_shared_boundary_as_overlap() {
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let boundary = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
        let next = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();

        assert!(date_ranges_overlap(
            start,
            Some(boundary),
            boundary,
            Some(next)
        ));
        assert!(!date_ranges_overlap(start, Some(boundary), next, None));
    }

    #[test]
    fn ledes_validation_only_flags_fee_items_missing_utbms_codes() {
        let now = chrono::DateTime::<chrono::Utc>::from_timestamp(1_762_476_800, 0).unwrap();
        let errors = validate_ledes98b_line_items(&[
            InvoiceLineItemRecord {
                id: Uuid::new_v4(),
                user_id: "test-user".to_string(),
                invoice_id: Uuid::new_v4(),
                description: "Time entry".to_string(),
                quantity: dec!(1.0),
                unit_price: dec!(300.0),
                amount: dec!(300.0),
                time_entry_id: Some(Uuid::new_v4()),
                expense_entry_id: None,
                task_code: None,
                activity_code: Some("A101".to_string()),
                timekeeper: Some("Lead".to_string()),
                resolved_rate: Some(dec!(300.0)),
                rate_source: Some(BillingRateSource::TimekeeperDefault),
                sort_order: 0,
                created_at: now,
                updated_at: now,
            },
            InvoiceLineItemRecord {
                id: Uuid::new_v4(),
                user_id: "test-user".to_string(),
                invoice_id: Uuid::new_v4(),
                description: "Expense entry".to_string(),
                quantity: dec!(1.0),
                unit_price: dec!(50.0),
                amount: dec!(50.0),
                time_entry_id: None,
                expense_entry_id: Some(Uuid::new_v4()),
                task_code: None,
                activity_code: None,
                timekeeper: None,
                resolved_rate: None,
                rate_source: None,
                sort_order: 1,
                created_at: now,
                updated_at: now,
            },
        ]);

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].missing_fields, vec![LedesRequiredField::TaskCode]);
    }
}
