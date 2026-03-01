use std::collections::HashSet;

use chrono::{NaiveDate, Utc};
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::{
    CreateInvoiceLineItemParams, CreateInvoiceParams, CreateTrustLedgerEntryParams, Database,
    InvoiceLineItemRecord, InvoiceRecord, InvoiceStatus, TrustLedgerEntryRecord,
    TrustLedgerEntryType,
};
use crate::error::DatabaseError;

#[derive(Debug, Clone)]
pub struct DraftInvoiceResult {
    pub invoice: CreateInvoiceParams,
    pub line_items: Vec<CreateInvoiceLineItemParams>,
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
        let unit_price = entry.hourly_rate.unwrap_or(Decimal::ZERO);
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
    db.save_invoice_draft(user_id, &draft.invoice, &draft.line_items)
        .await
}

pub async fn finalize_invoice(
    db: &dyn Database,
    user_id: &str,
    invoice_id: Uuid,
) -> Result<InvoiceRecord, String> {
    let invoice = db
        .get_invoice(user_id, invoice_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Invoice not found".to_string())?;
    if invoice.status != InvoiceStatus::Draft {
        return Err("Only draft invoices can be finalized".to_string());
    }

    let line_items = db
        .list_invoice_line_items(user_id, invoice_id)
        .await
        .map_err(|e| e.to_string())?;
    let mut time_ids = Vec::new();
    let mut expense_ids = Vec::new();
    let mut seen_time = HashSet::new();
    let mut seen_expense = HashSet::new();

    for item in line_items {
        if let Some(time_id) = item.time_entry_id
            && seen_time.insert(time_id)
        {
            time_ids.push(time_id);
        }
        if let Some(expense_id) = item.expense_entry_id
            && seen_expense.insert(expense_id)
        {
            expense_ids.push(expense_id);
        }
    }

    if !time_ids.is_empty() {
        db.mark_time_entries_billed(user_id, &time_ids, &invoice_id.to_string())
            .await
            .map_err(|e| e.to_string())?;
    }
    if !expense_ids.is_empty() {
        db.mark_expense_entries_billed(user_id, &expense_ids, &invoice_id.to_string())
            .await
            .map_err(|e| e.to_string())?;
    }

    db.set_invoice_status(
        user_id,
        invoice_id,
        InvoiceStatus::Sent,
        Some(Utc::now().date_naive()),
    )
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

    let invoice = db
        .get_invoice(user_id, invoice_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Invoice not found".to_string())?;
    if !matches!(invoice.status, InvoiceStatus::Sent) {
        return Err(format!(
            "Cannot record payment for invoice with status '{}'",
            invoice.status.as_str()
        ));
    }

    let trust_entry = if draw_from_trust {
        let entry = db
            .append_trust_ledger_entry(
                user_id,
                &invoice.matter_id,
                &CreateTrustLedgerEntryParams {
                    entry_type: TrustLedgerEntryType::InvoicePayment,
                    amount,
                    delta: -amount,
                    description: description
                        .unwrap_or("Invoice payment from trust")
                        .trim()
                        .to_string(),
                    invoice_id: Some(invoice_id),
                    recorded_by: recorded_by.trim().to_string(),
                },
            )
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg
                    .to_ascii_lowercase()
                    .contains("insufficient trust balance")
                {
                    "Trust balance is insufficient for this payment".to_string()
                } else {
                    msg
                }
            })?;
        Some(entry)
    } else {
        None
    };

    let updated = db
        .apply_invoice_payment(user_id, invoice_id, amount)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Invoice not found".to_string())?;
    Ok((updated, trust_entry))
}

pub async fn record_trust_deposit(
    db: &dyn Database,
    user_id: &str,
    matter_id: &str,
    amount: Decimal,
    recorded_by: &str,
    description: &str,
) -> Result<TrustLedgerEntryRecord, String> {
    if amount <= Decimal::ZERO {
        return Err("Deposit amount must be greater than 0".to_string());
    }
    db.append_trust_ledger_entry(
        user_id,
        matter_id,
        &CreateTrustLedgerEntryParams {
            entry_type: TrustLedgerEntryType::Deposit,
            amount,
            delta: amount,
            description: description.trim().to_string(),
            invoice_id: None,
            recorded_by: recorded_by.trim().to_string(),
        },
    )
    .await
    .map_err(|e| e.to_string())
}
