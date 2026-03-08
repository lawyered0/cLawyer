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
        task_code: entry.task_code,
        activity_code: entry.activity_code,
        resolved_rate: entry.resolved_rate.map(|value| value.to_string()),
        rate_source: entry.rate_source.map(|value| value.as_str().to_string()),
        entry_date: entry.entry_date.to_string(),
        billable: entry.billable,
        block_billing_flag: entry.block_billing_flag,
        block_billing_reason: entry.block_billing_reason,
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
        task_code: item.task_code,
        activity_code: item.activity_code,
        timekeeper: item.timekeeper,
        resolved_rate: item.resolved_rate.map(|value| value.to_string()),
        rate_source: item.rate_source.map(|value| value.as_str().to_string()),
        sort_order: item.sort_order,
    }
}

pub(crate) fn invoice_draft_line_item_to_info(
    item: &crate::legal::billing::DraftInvoiceLineItem,
) -> InvoiceDraftLineItemInfo {
    let params = &item.params;
    InvoiceDraftLineItemInfo {
        id: "draft".to_string(),
        description: params.description.clone(),
        quantity: params.quantity.to_string(),
        unit_price: params.unit_price.to_string(),
        amount: params.amount.to_string(),
        time_entry_id: params.time_entry_id.map(|value| value.to_string()),
        expense_entry_id: params.expense_entry_id.map(|value| value.to_string()),
        task_code: params.task_code.clone(),
        activity_code: params.activity_code.clone(),
        timekeeper: params.timekeeper.clone(),
        resolved_rate: params.resolved_rate.map(|value| value.to_string()),
        rate_source: params.rate_source.map(|value| value.as_str().to_string()),
        sort_order: params.sort_order,
        rate_resolution: item
            .rate_resolution
            .as_ref()
            .map(invoice_draft_rate_resolution_to_info),
    }
}

fn invoice_draft_rate_resolution_to_info(
    resolution: &crate::legal::billing::ResolvedTimeEntryRate,
) -> InvoiceDraftRateResolutionInfo {
    InvoiceDraftRateResolutionInfo {
        matched_schedule: resolution
            .matched_schedule
            .as_ref()
            .map(invoice_draft_rate_schedule_to_info),
        fallback_applied: resolution.fallback.is_some(),
        fallback_reason: resolution.fallback.map(|value| value.as_str().to_string()),
    }
}

fn invoice_draft_rate_schedule_to_info(
    schedule: &crate::db::BillingRateScheduleRecord,
) -> InvoiceDraftRateScheduleInfo {
    InvoiceDraftRateScheduleInfo {
        id: schedule.id.to_string(),
        matter_id: schedule.matter_id.clone(),
        timekeeper: schedule.timekeeper.clone(),
        rate: schedule.rate.to_string(),
        effective_start: schedule.effective_start.to_string(),
        effective_end: schedule.effective_end.map(|value| value.to_string()),
    }
}

pub(crate) fn trust_ledger_entry_record_to_info(
    entry: TrustLedgerEntryRecord,
) -> TrustLedgerEntryInfo {
    TrustLedgerEntryInfo {
        id: entry.id.to_string(),
        matter_id: entry.matter_id,
        trust_account_id: entry.trust_account_id.map(|value| value.to_string()),
        entry_type: entry.entry_type.as_str().to_string(),
        amount: entry.amount.to_string(),
        delta: entry.delta.to_string(),
        balance_after: entry.balance_after.to_string(),
        description: entry.description,
        reference_number: entry.reference_number,
        source: entry.source.as_str().to_string(),
        invoice_id: entry.invoice_id.map(|value| value.to_string()),
        recorded_by: entry.recorded_by,
        created_at: entry.created_at.to_rfc3339(),
    }
}

pub(crate) fn trust_account_record_to_info(
    account: crate::db::TrustAccountRecord,
    current_balance: Option<rust_decimal::Decimal>,
) -> TrustAccountInfo {
    TrustAccountInfo {
        id: account.id.to_string(),
        name: account.name,
        bank_name: account.bank_name,
        account_number_last4: account.account_number_last4,
        is_primary: account.is_primary,
        current_balance: current_balance.map(|value| value.to_string()),
        created_at: account.created_at.to_rfc3339(),
        updated_at: account.updated_at.to_rfc3339(),
    }
}

pub(crate) fn trust_statement_import_record_to_info(
    record: crate::db::TrustStatementImportRecord,
) -> TrustStatementImportInfo {
    TrustStatementImportInfo {
        id: record.id.to_string(),
        trust_account_id: record.trust_account_id.to_string(),
        statement_date: record.statement_date.to_string(),
        starting_balance: record.starting_balance.to_string(),
        ending_balance: record.ending_balance.to_string(),
        imported_by: record.imported_by,
        row_count: record.row_count,
        created_at: record.created_at.to_rfc3339(),
    }
}

pub(crate) fn trust_statement_line_record_to_info(
    record: crate::db::TrustStatementLineRecord,
) -> TrustStatementLineInfo {
    TrustStatementLineInfo {
        id: record.id.to_string(),
        entry_date: record.entry_date.to_string(),
        description: record.description,
        debit: record.debit.to_string(),
        credit: record.credit.to_string(),
        running_balance: record.running_balance.to_string(),
        reference_number: record.reference_number,
    }
}

pub(crate) fn trust_reconciliation_record_to_info(
    record: crate::db::TrustReconciliationRecord,
    report_markdown: Option<String>,
) -> TrustReconciliationInfo {
    TrustReconciliationInfo {
        id: record.id.to_string(),
        trust_account_id: record.trust_account_id.to_string(),
        statement_import_id: record.statement_import_id.to_string(),
        statement_ending_balance: record.statement_ending_balance.to_string(),
        book_balance: record.book_balance.to_string(),
        client_balance_total: record.client_balance_total.to_string(),
        difference: record.difference.to_string(),
        exceptions_json: record.exceptions_json,
        status: record.status.as_str().to_string(),
        signed_off_by: record.signed_off_by,
        signed_off_at: record.signed_off_at.map(|value| value.to_rfc3339()),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        report_markdown,
    }
}

pub(crate) fn billing_rate_schedule_record_to_info(
    record: crate::db::BillingRateScheduleRecord,
) -> BillingRateScheduleInfo {
    BillingRateScheduleInfo {
        id: record.id.to_string(),
        matter_id: record.matter_id,
        timekeeper: record.timekeeper,
        rate: record.rate.to_string(),
        effective_start: record.effective_start.to_string(),
        effective_end: record.effective_end.map(|value| value.to_string()),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_party_record_to_info(record: crate::db::MatterPartyRecord) -> MatterPartyInfo {
    MatterPartyInfo {
        id: record.id.to_string(),
        matter_id: record.matter_id,
        party_id: record.party_id.to_string(),
        name: record.name,
        role: record.role.as_str().to_string(),
        aliases: record.aliases,
        notes: record.notes,
        opened_at: record.opened_at.map(|value| value.to_rfc3339()),
        closed_at: record.closed_at.map(|value| value.to_rfc3339()),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(crate) fn party_relationship_record_to_info(
    record: crate::db::PartyRelationshipRecord,
) -> PartyRelationshipInfo {
    PartyRelationshipInfo {
        id: record.id.to_string(),
        parent_party_id: record.parent_party_id.to_string(),
        parent_name: record.parent_name,
        child_party_id: record.child_party_id.to_string(),
        child_name: record.child_name,
        kind: record.kind,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(crate) fn conflict_clearance_info_to_response(
    info: crate::db::ConflictClearanceInfo,
) -> ConflictClearanceInfoResponse {
    ConflictClearanceInfoResponse {
        matter_id: info.matter_id,
        checked_by: info.checked_by,
        cleared_by: info.cleared_by,
        decision: info.decision.as_str().to_string(),
        note: info.note,
        hit_count: info.hit_count,
        reviewing_attorney: info.reviewing_attorney,
        report_hash: info.report_hash,
        signed_at: info.signed_at.map(|value| value.to_rfc3339()),
        created_at: info.created_at.to_rfc3339(),
    }
}

pub(crate) fn citation_verification_run_record_to_info(
    record: crate::db::CitationVerificationRunRecord,
) -> CitationVerificationRunInfo {
    CitationVerificationRunInfo {
        id: record.id.to_string(),
        matter_id: record.matter_id,
        matter_document_id: record.matter_document_id.to_string(),
        provider: record.provider,
        document_hash: record.document_hash,
        created_by: record.created_by,
        created_at: record.created_at.to_rfc3339(),
    }
}

pub(crate) fn citation_verification_result_record_to_info(
    record: crate::db::CitationVerificationResultRecord,
) -> CitationVerificationResultInfo {
    CitationVerificationResultInfo {
        id: record.id.to_string(),
        citation_text: record.citation_text,
        normalized_citation: record.normalized_citation,
        status: record.status.as_str().to_string(),
        provider_reference: record.provider_reference,
        provider_title: record.provider_title,
        detail: record.detail,
        waived_by: record.waived_by,
        waiver_reason: record.waiver_reason,
        waived_at: record.waived_at.map(|value| value.to_rfc3339()),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
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
