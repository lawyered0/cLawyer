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

#[derive(Debug, Clone)]
pub struct DraftInvoiceResult {
    pub invoice: CreateInvoiceParams,
    pub line_items: Vec<CreateInvoiceLineItemParams>,
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

pub async fn resolve_time_entry_rate(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    timekeeper: &str,
    entry_date: NaiveDate,
    manual_hourly_rate: Option<Decimal>,
) -> Result<(Option<Decimal>, Option<BillingRateSource>), DatabaseError> {
    let matter_schedules = db
        .list_billing_rate_schedules(user_id, Some(matter_id), Some(timekeeper))
        .await?;
    if let Some(schedule) = best_schedule(matter_schedules.iter(), entry_date) {
        return Ok((Some(schedule.rate), Some(BillingRateSource::MatterOverride)));
    }

    let default_schedules = db
        .list_billing_rate_schedules(user_id, None, Some(timekeeper))
        .await?;
    if let Some(schedule) = best_schedule(default_schedules.iter(), entry_date) {
        return Ok((
            Some(schedule.rate),
            Some(BillingRateSource::TimekeeperDefault),
        ));
    }

    Ok((
        manual_hourly_rate,
        manual_hourly_rate.map(|_| BillingRateSource::ManualOverride),
    ))
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
        let (resolved_rate, rate_source) = match (entry.resolved_rate, entry.rate_source) {
            (Some(rate), source) => (Some(rate), source),
            _ => {
                resolve_time_entry_rate(
                    db,
                    user_id,
                    matter_id,
                    &entry.timekeeper,
                    entry.entry_date,
                    entry.hourly_rate,
                )
                .await?
            }
        };
        let unit_price = resolved_rate.unwrap_or(Decimal::ZERO);
        let amount = (entry.hours * unit_price).round_dp(2);
        line_items.push(CreateInvoiceLineItemParams {
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
        });
    }

    for entry in expense_entries {
        if entry.billed_invoice_id.is_some() || !entry.billable {
            continue;
        }
        line_items.push(CreateInvoiceLineItemParams {
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
        });
    }

    let subtotal = line_items
        .iter()
        .fold(Decimal::ZERO, |acc, item| acc + item.amount)
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
    validate_invoice_totals(&draft.invoice, &draft.line_items)?;
    db.save_invoice_draft(user_id, &draft.invoice, &draft.line_items)
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
}
