//! Record-to-response mapping helpers for web handlers.

use crate::channels::web::types::*;
use crate::db::{InvoiceLineItemRecord, InvoiceRecord, TrustLedgerEntryRecord};

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
