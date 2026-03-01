use chrono::{DateTime, NaiveDate, Utc};
use libsql::params;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::{
    BillingStore, ClientRecord, ClientStore, ClientType, CreateClientParams,
    CreateDocumentVersionParams, CreateExpenseEntryParams, CreateInvoiceLineItemParams,
    CreateInvoiceParams, CreateMatterDeadlineParams, CreateMatterNoteParams,
    CreateMatterTaskParams, CreateTimeEntryParams, CreateTrustLedgerEntryParams,
    DocumentTemplateRecord, DocumentTemplateStore, DocumentVersionRecord, DocumentVersionStore,
    ExpenseCategory, ExpenseEntryRecord, InvoiceLineItemRecord, InvoiceRecord, InvoiceStatus,
    MatterDeadlineRecord, MatterDeadlineStore, MatterDeadlineType, MatterDocumentCategory,
    MatterDocumentRecord, MatterDocumentStore, MatterNoteRecord, MatterNoteStore, MatterRecord,
    MatterStatus, MatterStore, MatterTaskRecord, MatterTaskStatus, MatterTaskStore,
    MatterTimeSummary, TimeEntryRecord, TimeExpenseStore, TrustLedgerEntryRecord,
    TrustLedgerEntryType, UpdateClientParams, UpdateDocumentTemplateParams,
    UpdateExpenseEntryParams, UpdateMatterDeadlineParams, UpdateMatterDocumentParams,
    UpdateMatterNoteParams, UpdateMatterParams, UpdateMatterTaskParams, UpdateTimeEntryParams,
    UpsertDocumentTemplateParams, UpsertMatterDocumentParams, UpsertMatterParams,
    normalize_party_name,
};
use crate::error::DatabaseError;

use super::{
    LibSqlBackend, fmt_ts, get_decimal, get_i64, get_opt_decimal, get_opt_text, get_text, opt_text,
    opt_text_owned, parse_timestamp,
};

fn parse_uuid(raw: &str, field: &str) -> Result<Uuid, DatabaseError> {
    Uuid::parse_str(raw)
        .map_err(|e| DatabaseError::Serialization(format!("invalid {} uuid: {}", field, e)))
}

fn parse_dt_opt(raw: Option<String>) -> Result<Option<DateTime<Utc>>, DatabaseError> {
    match raw {
        Some(value) => parse_timestamp(&value)
            .map(Some)
            .map_err(|e| DatabaseError::Serialization(e.to_string())),
        None => Ok(None),
    }
}

fn parse_naive_date(raw: &str, field: &str) -> Result<NaiveDate, DatabaseError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| {
        DatabaseError::Serialization(format!(
            "invalid {} date '{}'; expected YYYY-MM-DD",
            field, raw
        ))
    })
}

fn parse_client_type(raw: &str) -> Result<ClientType, DatabaseError> {
    ClientType::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid client_type '{}'", raw)))
}

fn parse_matter_status(raw: &str) -> Result<MatterStatus, DatabaseError> {
    MatterStatus::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid matter status '{}'", raw)))
}

fn parse_matter_task_status(raw: &str) -> Result<MatterTaskStatus, DatabaseError> {
    MatterTaskStatus::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter task status '{}'", raw))
    })
}

fn parse_matter_deadline_type(raw: &str) -> Result<MatterDeadlineType, DatabaseError> {
    MatterDeadlineType::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter deadline type '{}'", raw))
    })
}

fn parse_expense_category(raw: &str) -> Result<ExpenseCategory, DatabaseError> {
    ExpenseCategory::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid expense category '{}'", raw)))
}

fn parse_invoice_status(raw: &str) -> Result<InvoiceStatus, DatabaseError> {
    InvoiceStatus::from_db_value(raw)
        .ok_or_else(|| DatabaseError::Serialization(format!("invalid invoice status '{}'", raw)))
}

fn parse_trust_ledger_entry_type(raw: &str) -> Result<TrustLedgerEntryType, DatabaseError> {
    TrustLedgerEntryType::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid trust ledger entry type '{}'", raw))
    })
}

fn parse_json_array_strings(raw: &str) -> Result<Vec<String>, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(parsed
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default())
}

fn parse_json_array_uuids(raw: &str) -> Result<Vec<Uuid>, DatabaseError> {
    let values = parse_json_array_strings(raw)?;
    values
        .into_iter()
        .map(|value| parse_uuid(&value, "blocked_by"))
        .collect()
}

fn parse_json_object(raw: &str) -> Result<serde_json::Value, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!({}));
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Ok(serde_json::json!({}))
    }
}

fn parse_json_array_i32(raw: &str) -> Result<Vec<i32>, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(parsed
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|entry| entry.as_i64())
                .filter_map(|value| i32::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default())
}

fn row_to_client_record(row: &libsql::Row) -> Result<ClientRecord, DatabaseError> {
    let client_type_raw = get_text(row, 4);
    Ok(ClientRecord {
        id: parse_uuid(&get_text(row, 0), "client.id")?,
        user_id: get_text(row, 1),
        name: get_text(row, 2),
        name_normalized: get_text(row, 3),
        client_type: parse_client_type(&client_type_raw)?,
        email: get_opt_text(row, 5),
        phone: get_opt_text(row, 6),
        address: get_opt_text(row, 7),
        notes: get_opt_text(row, 8),
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_record(row: &libsql::Row) -> Result<MatterRecord, DatabaseError> {
    let status_raw = get_text(row, 3);
    let opened_at = parse_dt_opt(get_opt_text(row, 7))?;
    let closed_at = parse_dt_opt(get_opt_text(row, 8))?;

    Ok(MatterRecord {
        user_id: get_text(row, 0),
        matter_id: get_text(row, 1),
        client_id: parse_uuid(&get_text(row, 2), "matters.client_id")?,
        status: parse_matter_status(&status_raw)?,
        stage: get_opt_text(row, 4),
        practice_area: get_opt_text(row, 5),
        jurisdiction: get_opt_text(row, 6),
        opened_at,
        closed_at,
        assigned_to: parse_json_array_strings(&get_text(row, 9))?,
        custom_fields: parse_json_object(&get_text(row, 10))?,
        created_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_task_record(row: &libsql::Row) -> Result<MatterTaskRecord, DatabaseError> {
    let status_raw = get_text(row, 5);
    let due_at = parse_dt_opt(get_opt_text(row, 7))?;
    Ok(MatterTaskRecord {
        id: parse_uuid(&get_text(row, 0), "matter_tasks.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        title: get_text(row, 3),
        description: get_opt_text(row, 4),
        status: parse_matter_task_status(&status_raw)?,
        assignee: get_opt_text(row, 6),
        due_at,
        blocked_by: parse_json_array_uuids(&get_text(row, 8))?,
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_note_record(row: &libsql::Row) -> Result<MatterNoteRecord, DatabaseError> {
    Ok(MatterNoteRecord {
        id: parse_uuid(&get_text(row, 0), "matter_notes.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        author: get_text(row, 3),
        body: get_text(row, 4),
        pinned: get_i64(row, 5) != 0,
        created_at: parse_timestamp(&get_text(row, 6))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_matter_deadline_record(row: &libsql::Row) -> Result<MatterDeadlineRecord, DatabaseError> {
    let deadline_type_raw = get_text(row, 4);
    let due_at = parse_timestamp(&get_text(row, 5))
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    let completed_at = parse_dt_opt(get_opt_text(row, 6))?;
    Ok(MatterDeadlineRecord {
        id: parse_uuid(&get_text(row, 0), "matter_deadlines.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        title: get_text(row, 3),
        deadline_type: parse_matter_deadline_type(&deadline_type_raw)?,
        due_at,
        completed_at,
        reminder_days: parse_json_array_i32(&get_text(row, 7))?,
        rule_ref: get_opt_text(row, 8),
        computed_from: get_opt_text(row, 9)
            .map(|value| parse_uuid(&value, "matter_deadlines.computed_from"))
            .transpose()?,
        task_id: get_opt_text(row, 10)
            .map(|value| parse_uuid(&value, "matter_deadlines.task_id"))
            .transpose()?,
        created_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn parse_matter_document_category(raw: &str) -> Result<MatterDocumentCategory, DatabaseError> {
    MatterDocumentCategory::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid matter document category '{}'", raw))
    })
}

fn row_to_matter_document_record(row: &libsql::Row) -> Result<MatterDocumentRecord, DatabaseError> {
    let category_raw = get_text(row, 6);
    Ok(MatterDocumentRecord {
        id: parse_uuid(&get_text(row, 0), "matter_documents.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        memory_document_id: parse_uuid(&get_text(row, 3), "matter_documents.memory_document_id")?,
        path: get_text(row, 4),
        display_name: get_text(row, 5),
        category: parse_matter_document_category(&category_raw)?,
        created_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 8))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_document_version_record(
    row: &libsql::Row,
) -> Result<DocumentVersionRecord, DatabaseError> {
    Ok(DocumentVersionRecord {
        id: parse_uuid(&get_text(row, 0), "document_versions.id")?,
        user_id: get_text(row, 1),
        matter_document_id: parse_uuid(&get_text(row, 2), "document_versions.matter_document_id")?,
        version_number: i32::try_from(get_i64(row, 3))
            .map_err(|_| DatabaseError::Serialization("invalid version_number".to_string()))?,
        label: get_text(row, 4),
        memory_document_id: parse_uuid(&get_text(row, 5), "document_versions.memory_document_id")?,
        created_at: parse_timestamp(&get_text(row, 6))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_document_template_record(
    row: &libsql::Row,
) -> Result<DocumentTemplateRecord, DatabaseError> {
    let variables_json = parse_json_object_or_array(&get_text(row, 5))?;
    Ok(DocumentTemplateRecord {
        id: parse_uuid(&get_text(row, 0), "document_templates.id")?,
        user_id: get_text(row, 1),
        matter_id: get_opt_text(row, 2),
        name: get_text(row, 3),
        body: get_text(row, 4),
        variables_json,
        created_at: parse_timestamp(&get_text(row, 6))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_time_entry_record(row: &libsql::Row) -> Result<TimeEntryRecord, DatabaseError> {
    let entry_date_raw = get_text(row, 7);
    Ok(TimeEntryRecord {
        id: parse_uuid(&get_text(row, 0), "time_entries.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        timekeeper: get_text(row, 3),
        description: get_text(row, 4),
        hours: get_decimal(row, 5),
        hourly_rate: get_opt_decimal(row, 6),
        entry_date: parse_naive_date(&entry_date_raw, "entry_date")?,
        billable: get_i64(row, 8) != 0,
        billed_invoice_id: get_opt_text(row, 9),
        created_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_expense_entry_record(row: &libsql::Row) -> Result<ExpenseEntryRecord, DatabaseError> {
    let category_raw = get_text(row, 6);
    let entry_date_raw = get_text(row, 7);
    Ok(ExpenseEntryRecord {
        id: parse_uuid(&get_text(row, 0), "expense_entries.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        submitted_by: get_text(row, 3),
        description: get_text(row, 4),
        amount: get_decimal(row, 5),
        category: parse_expense_category(&category_raw)?,
        entry_date: parse_naive_date(&entry_date_raw, "entry_date")?,
        receipt_path: get_opt_text(row, 8),
        billable: get_i64(row, 9) != 0,
        billed_invoice_id: get_opt_text(row, 10),
        created_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_invoice_record(row: &libsql::Row) -> Result<InvoiceRecord, DatabaseError> {
    let status_raw = get_text(row, 4);
    let issued_date = get_opt_text(row, 5)
        .map(|raw| parse_naive_date(&raw, "issued_date"))
        .transpose()?;
    let due_date = get_opt_text(row, 6)
        .map(|raw| parse_naive_date(&raw, "due_date"))
        .transpose()?;
    Ok(InvoiceRecord {
        id: parse_uuid(&get_text(row, 0), "invoices.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        invoice_number: get_text(row, 3),
        status: parse_invoice_status(&status_raw)?,
        issued_date,
        due_date,
        subtotal: get_decimal(row, 7),
        tax: get_decimal(row, 8),
        total: get_decimal(row, 9),
        paid_amount: get_decimal(row, 10),
        notes: get_opt_text(row, 11),
        created_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 13))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_invoice_line_item_record(
    row: &libsql::Row,
) -> Result<InvoiceLineItemRecord, DatabaseError> {
    Ok(InvoiceLineItemRecord {
        id: parse_uuid(&get_text(row, 0), "invoice_line_items.id")?,
        user_id: get_text(row, 1),
        invoice_id: parse_uuid(&get_text(row, 2), "invoice_line_items.invoice_id")?,
        description: get_text(row, 3),
        quantity: get_decimal(row, 4),
        unit_price: get_decimal(row, 5),
        amount: get_decimal(row, 6),
        time_entry_id: get_opt_text(row, 7)
            .map(|value| parse_uuid(&value, "invoice_line_items.time_entry_id"))
            .transpose()?,
        expense_entry_id: get_opt_text(row, 8)
            .map(|value| parse_uuid(&value, "invoice_line_items.expense_entry_id"))
            .transpose()?,
        sort_order: i32::try_from(get_i64(row, 9))
            .map_err(|_| DatabaseError::Serialization("invalid sort_order".to_string()))?,
        created_at: parse_timestamp(&get_text(row, 10))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_ledger_entry_record(
    row: &libsql::Row,
) -> Result<TrustLedgerEntryRecord, DatabaseError> {
    let entry_type_raw = get_text(row, 3);
    Ok(TrustLedgerEntryRecord {
        id: parse_uuid(&get_text(row, 0), "trust_ledger.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        entry_type: parse_trust_ledger_entry_type(&entry_type_raw)?,
        amount: get_decimal(row, 4),
        balance_after: get_decimal(row, 5),
        description: get_text(row, 6),
        invoice_id: get_opt_text(row, 7)
            .map(|value| parse_uuid(&value, "trust_ledger.invoice_id"))
            .transpose()?,
        recorded_by: get_text(row, 8),
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn parse_json_object_or_array(raw: &str) -> Result<serde_json::Value, DatabaseError> {
    if raw.trim().is_empty() {
        return Ok(serde_json::json!([]));
    }
    let value: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    if value.is_object() || value.is_array() {
        Ok(value)
    } else {
        Ok(serde_json::json!([]))
    }
}

#[async_trait::async_trait]
impl ClientStore for LibSqlBackend {
    async fn create_client(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO clients (id, user_id, name, name_normalized, client_type, email, phone, address, notes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id.as_str(),
                user_id,
                input.name.trim(),
                normalized_name.as_str(),
                input.client_type.as_str(),
                opt_text(input.email.as_deref()),
                opt_text(input.phone.as_deref()),
                opt_text(input.address.as_deref()),
                opt_text(input.notes.as_deref()),
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE id = ?1 LIMIT 1",
                params![id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load created client".to_string()))?;

        row_to_client_record(&row)
    }

    async fn upsert_client_by_normalized_name(
        &self,
        user_id: &str,
        input: &CreateClientParams,
    ) -> Result<ClientRecord, DatabaseError> {
        let normalized_name = normalize_party_name(&input.name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }

        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO clients (id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now')) \
             ON CONFLICT (user_id, name_normalized) DO UPDATE SET \
               name = excluded.name, \
               client_type = excluded.client_type, \
               email = COALESCE(excluded.email, clients.email), \
               phone = COALESCE(excluded.phone, clients.phone), \
               address = COALESCE(excluded.address, clients.address), \
               notes = COALESCE(excluded.notes, clients.notes), \
               updated_at = datetime('now')",
            params![
                Uuid::new_v4().to_string(),
                user_id,
                input.name.trim(),
                normalized_name.as_str(),
                input.client_type.as_str(),
                opt_text(input.email.as_deref()),
                opt_text(input.phone.as_deref()),
                opt_text(input.address.as_deref()),
                opt_text(input.notes.as_deref()),
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 AND name_normalized = ?2 LIMIT 1",
                params![user_id, normalized_name.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to resolve upserted client".to_string()))?;

        row_to_client_record(&row)
    }

    async fn list_clients(
        &self,
        user_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClientRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(search_raw) = query {
            let search = normalize_party_name(search_raw);
            if search.is_empty() {
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients WHERE user_id = ?1 ORDER BY name ASC",
                    params![user_id],
                )
                .await?
            } else {
                let like = format!("%{}%", search);
                conn.query(
                    "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                     FROM clients WHERE user_id = ?1 AND name_normalized LIKE ?2 ORDER BY name ASC",
                    params![user_id, like],
                )
                .await?
            }
        } else {
            conn.query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 ORDER BY name ASC",
                params![user_id],
            )
            .await?
        };

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_client_record(&row)?);
        }
        Ok(out)
    }

    async fn get_client(
        &self,
        user_id: &str,
        client_id: Uuid,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, name, name_normalized, client_type, email, phone, address, notes, created_at, updated_at \
                 FROM clients WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, client_id.to_string()],
            )
            .await?
            .next()
            .await?;

        row.map(|row| row_to_client_record(&row)).transpose()
    }

    async fn update_client(
        &self,
        user_id: &str,
        client_id: Uuid,
        input: &UpdateClientParams,
    ) -> Result<Option<ClientRecord>, DatabaseError> {
        let Some(existing) = self.get_client(user_id, client_id).await? else {
            return Ok(None);
        };

        let merged_name = input
            .name
            .as_deref()
            .unwrap_or(existing.name.as_str())
            .trim();
        let normalized_name = normalize_party_name(merged_name);
        if normalized_name.is_empty() {
            return Err(DatabaseError::Serialization(
                "client name cannot be empty".to_string(),
            ));
        }
        let merged_client_type = input.client_type.unwrap_or(existing.client_type);
        let merged_email = input.email.clone().unwrap_or(existing.email);
        let merged_phone = input.phone.clone().unwrap_or(existing.phone);
        let merged_address = input.address.clone().unwrap_or(existing.address);
        let merged_notes = input.notes.clone().unwrap_or(existing.notes);

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE clients SET \
               name = ?3, \
               name_normalized = ?4, \
               client_type = ?5, \
               email = ?6, \
               phone = ?7, \
               address = ?8, \
               notes = ?9, \
               updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                client_id.to_string(),
                merged_name,
                normalized_name.as_str(),
                merged_client_type.as_str(),
                opt_text(merged_email.as_deref()),
                opt_text(merged_phone.as_deref()),
                opt_text(merged_address.as_deref()),
                opt_text(merged_notes.as_deref()),
            ],
        )
        .await?;

        self.get_client(user_id, client_id).await
    }

    async fn delete_client(&self, user_id: &str, client_id: Uuid) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM clients WHERE user_id = ?1 AND id = ?2",
                params![user_id, client_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterStore for LibSqlBackend {
    async fn upsert_matter(
        &self,
        user_id: &str,
        input: &UpsertMatterParams,
    ) -> Result<MatterRecord, DatabaseError> {
        let assigned_to = serde_json::to_string(&input.assigned_to)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
        let custom_fields = if input.custom_fields.is_object() {
            serde_json::to_string(&input.custom_fields)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?
        } else {
            "{}".to_string()
        };

        let conn = self.connect().await?;
        conn.execute(
            "INSERT INTO matters \
             (user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), datetime('now')) \
             ON CONFLICT (user_id, matter_id) DO UPDATE SET \
                client_id = excluded.client_id, \
                status = excluded.status, \
                stage = excluded.stage, \
                practice_area = excluded.practice_area, \
                jurisdiction = excluded.jurisdiction, \
                opened_at = excluded.opened_at, \
                closed_at = excluded.closed_at, \
                assigned_to = excluded.assigned_to, \
                custom_fields = excluded.custom_fields, \
                updated_at = datetime('now')",
            params![
                user_id,
                input.matter_id.as_str(),
                input.client_id.to_string(),
                input.status.as_str(),
                opt_text(input.stage.as_deref()),
                opt_text(input.practice_area.as_deref()),
                opt_text(input.jurisdiction.as_deref()),
                opt_text_owned(input.opened_at.as_ref().map(fmt_ts)),
                opt_text_owned(input.closed_at.as_ref().map(fmt_ts)),
                assigned_to,
                custom_fields,
            ],
        )
        .await?;

        self.get_matter_db(user_id, &input.matter_id)
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to resolve upserted matter".to_string()))
    }

    async fn list_matters_db(&self, user_id: &str) -> Result<Vec<MatterRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters WHERE user_id = ?1 ORDER BY matter_id ASC",
                params![user_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_record(&row)?);
        }
        Ok(out)
    }

    async fn get_matter_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT user_id, matter_id, client_id, status, stage, practice_area, jurisdiction, opened_at, closed_at, assigned_to, custom_fields, created_at, updated_at \
                 FROM matters WHERE user_id = ?1 AND matter_id = ?2 LIMIT 1",
                params![user_id, matter_id],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_matter_record(&row)).transpose()
    }

    async fn update_matter(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpdateMatterParams,
    ) -> Result<Option<MatterRecord>, DatabaseError> {
        let Some(existing) = self.get_matter_db(user_id, matter_id).await? else {
            return Ok(None);
        };

        let merged = UpsertMatterParams {
            matter_id: existing.matter_id.clone(),
            client_id: input.client_id.unwrap_or(existing.client_id),
            status: input.status.unwrap_or(existing.status),
            stage: input.stage.clone().unwrap_or(existing.stage),
            practice_area: input
                .practice_area
                .clone()
                .unwrap_or(existing.practice_area),
            jurisdiction: input.jurisdiction.clone().unwrap_or(existing.jurisdiction),
            opened_at: input.opened_at.unwrap_or(existing.opened_at),
            closed_at: input.closed_at.unwrap_or(existing.closed_at),
            assigned_to: input.assigned_to.clone().unwrap_or(existing.assigned_to),
            custom_fields: input
                .custom_fields
                .clone()
                .unwrap_or(existing.custom_fields),
        };

        let updated = self.upsert_matter(user_id, &merged).await?;
        Ok(Some(updated))
    }

    async fn delete_matter(&self, user_id: &str, matter_id: &str) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matters WHERE user_id = ?1 AND matter_id = ?2",
                params![user_id, matter_id],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterTaskStore for LibSqlBackend {
    async fn list_matter_tasks(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterTaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 ORDER BY created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_task_record(&row)?);
        }
        Ok(out)
    }

    async fn create_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterTaskParams,
    ) -> Result<MatterTaskRecord, DatabaseError> {
        let blocked_by = serde_json::to_string(
            &input
                .blocked_by
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.connect().await?;
        let task_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO matter_tasks \
             (id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
            params![
                task_id.to_string(),
                user_id,
                matter_id,
                input.title.as_str(),
                opt_text(input.description.as_deref()),
                input.status.as_str(),
                opt_text(input.assignee.as_deref()),
                opt_text_owned(input.due_at.as_ref().map(fmt_ts)),
                blocked_by,
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load created matter task".to_string()))?;

        row_to_matter_task_record(&row)
    }

    async fn update_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
        input: &UpdateMatterTaskParams,
    ) -> Result<Option<MatterTaskRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let existing_row = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?;
        let Some(existing_row) = existing_row else {
            return Ok(None);
        };
        let existing = row_to_matter_task_record(&existing_row)?;

        let merged_title = input.title.clone().unwrap_or(existing.title);
        let merged_description = input.description.clone().unwrap_or(existing.description);
        let merged_status = input.status.unwrap_or(existing.status);
        let merged_assignee = input.assignee.clone().unwrap_or(existing.assignee);
        let merged_due_at = input.due_at.unwrap_or(existing.due_at);
        let merged_blocked_by = input.blocked_by.clone().unwrap_or(existing.blocked_by);
        let blocked_by = serde_json::to_string(
            &merged_blocked_by
                .into_iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        )
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            "UPDATE matter_tasks SET \
                title = ?4, \
                description = ?5, \
                status = ?6, \
                assignee = ?7, \
                due_at = ?8, \
                blocked_by = ?9, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                task_id.to_string(),
                merged_title,
                opt_text(merged_description.as_deref()),
                merged_status.as_str(),
                opt_text(merged_assignee.as_deref()),
                opt_text_owned(merged_due_at.as_ref().map(fmt_ts)),
                blocked_by,
            ],
        )
        .await?;

        let updated = conn
            .query(
                "SELECT id, user_id, matter_id, title, description, status, assignee, due_at, blocked_by, created_at, updated_at \
                 FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?
            .next()
            .await?;
        updated
            .map(|row| row_to_matter_task_record(&row))
            .transpose()
    }

    async fn delete_matter_task(
        &self,
        user_id: &str,
        matter_id: &str,
        task_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_tasks WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, task_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterNoteStore for LibSqlBackend {
    async fn list_matter_notes(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterNoteRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 \
                 ORDER BY pinned DESC, created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_note_record(&row)?);
        }
        Ok(out)
    }

    async fn create_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterNoteParams,
    ) -> Result<MatterNoteRecord, DatabaseError> {
        let conn = self.connect().await?;
        let note_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO matter_notes (id, user_id, matter_id, author, body, pinned, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
            params![
                note_id.to_string(),
                user_id,
                matter_id,
                input.author.as_str(),
                input.body.as_str(),
                if input.pinned { 1 } else { 0 },
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| {
                DatabaseError::Query("failed to load created matter note".to_string())
            })?;
        row_to_matter_note_record(&row)
    }

    async fn update_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
        input: &UpdateMatterNoteParams,
    ) -> Result<Option<MatterNoteRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let existing_row = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?;
        let Some(existing_row) = existing_row else {
            return Ok(None);
        };
        let existing = row_to_matter_note_record(&existing_row)?;

        let merged_author = input.author.clone().unwrap_or(existing.author);
        let merged_body = input.body.clone().unwrap_or(existing.body);
        let merged_pinned = input.pinned.unwrap_or(existing.pinned);

        conn.execute(
            "UPDATE matter_notes SET \
                author = ?4, \
                body = ?5, \
                pinned = ?6, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                note_id.to_string(),
                merged_author,
                merged_body,
                if merged_pinned { 1 } else { 0 },
            ],
        )
        .await?;

        let updated = conn
            .query(
                "SELECT id, user_id, matter_id, author, body, pinned, created_at, updated_at \
                 FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?
            .next()
            .await?;
        updated
            .map(|row| row_to_matter_note_record(&row))
            .transpose()
    }

    async fn delete_matter_note(
        &self,
        user_id: &str,
        matter_id: &str,
        note_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_notes WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, note_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterDeadlineStore for LibSqlBackend {
    async fn list_matter_deadlines(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterDeadlineRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at \
                 FROM matter_deadlines WHERE user_id = ?1 AND matter_id = ?2 \
                 ORDER BY due_at ASC, created_at ASC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_deadline_record(&row)?);
        }
        Ok(out)
    }

    async fn get_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at \
                 FROM matter_deadlines WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, deadline_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_matter_deadline_record(&row))
            .transpose()
    }

    async fn create_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateMatterDeadlineParams,
    ) -> Result<MatterDeadlineRecord, DatabaseError> {
        let conn = self.connect().await?;
        let deadline_id = Uuid::new_v4();
        let reminder_days = serde_json::to_string(&input.reminder_days)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        conn.execute(
            "INSERT INTO matter_deadlines \
             (id, user_id, matter_id, title, deadline_type, due_at, completed_at, reminder_days, rule_ref, computed_from, task_id, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), datetime('now'))",
            params![
                deadline_id.to_string(),
                user_id,
                matter_id,
                input.title.as_str(),
                input.deadline_type.as_str(),
                fmt_ts(&input.due_at),
                opt_text_owned(input.completed_at.as_ref().map(fmt_ts)),
                reminder_days,
                opt_text(input.rule_ref.as_deref()),
                opt_text_owned(input.computed_from.as_ref().map(Uuid::to_string)),
                opt_text_owned(input.task_id.as_ref().map(Uuid::to_string)),
            ],
        )
        .await?;

        self.get_matter_deadline(user_id, matter_id, deadline_id)
            .await?
            .ok_or_else(|| {
                DatabaseError::Query("failed to load created matter deadline".to_string())
            })
    }

    async fn update_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
        input: &UpdateMatterDeadlineParams,
    ) -> Result<Option<MatterDeadlineRecord>, DatabaseError> {
        let Some(existing) = self
            .get_matter_deadline(user_id, matter_id, deadline_id)
            .await?
        else {
            return Ok(None);
        };

        let merged_title = input.title.clone().unwrap_or(existing.title);
        let merged_deadline_type = input.deadline_type.unwrap_or(existing.deadline_type);
        let merged_due_at = input.due_at.unwrap_or(existing.due_at);
        let merged_completed_at = input.completed_at.unwrap_or(existing.completed_at);
        let merged_reminder_days = input
            .reminder_days
            .clone()
            .unwrap_or(existing.reminder_days);
        let merged_rule_ref = input.rule_ref.clone().unwrap_or(existing.rule_ref);
        let merged_computed_from = input.computed_from.unwrap_or(existing.computed_from);
        let merged_task_id = input.task_id.unwrap_or(existing.task_id);
        let reminder_days = serde_json::to_string(&merged_reminder_days)
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?;

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE matter_deadlines SET \
                title = ?4, \
                deadline_type = ?5, \
                due_at = ?6, \
                completed_at = ?7, \
                reminder_days = ?8, \
                rule_ref = ?9, \
                computed_from = ?10, \
                task_id = ?11, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                deadline_id.to_string(),
                merged_title,
                merged_deadline_type.as_str(),
                fmt_ts(&merged_due_at),
                opt_text_owned(merged_completed_at.as_ref().map(fmt_ts)),
                reminder_days,
                opt_text(merged_rule_ref.as_deref()),
                opt_text_owned(merged_computed_from.as_ref().map(Uuid::to_string)),
                opt_text_owned(merged_task_id.as_ref().map(Uuid::to_string)),
            ],
        )
        .await?;

        self.get_matter_deadline(user_id, matter_id, deadline_id)
            .await
    }

    async fn delete_matter_deadline(
        &self,
        user_id: &str,
        matter_id: &str,
        deadline_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_deadlines WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, deadline_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl MatterDocumentStore for LibSqlBackend {
    async fn list_matter_documents_db(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<MatterDocumentRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT md.id, md.user_id, md.matter_id, md.memory_document_id, d.path, md.display_name, md.category, md.created_at, md.updated_at \
                 FROM matter_documents md \
                 JOIN memory_documents d ON d.id = md.memory_document_id \
                 WHERE md.user_id = ?1 AND md.matter_id = ?2 \
                 ORDER BY d.path ASC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_matter_document_record(&row)?);
        }
        Ok(out)
    }

    async fn get_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT md.id, md.user_id, md.matter_id, md.memory_document_id, d.path, md.display_name, md.category, md.created_at, md.updated_at \
                 FROM matter_documents md \
                 JOIN memory_documents d ON d.id = md.memory_document_id \
                 WHERE md.user_id = ?1 AND md.matter_id = ?2 AND md.id = ?3 LIMIT 1",
                params![user_id, matter_id, matter_document_id.to_string()],
            )
            .await?
            .next()
            .await?;

        row.map(|row| row_to_matter_document_record(&row))
            .transpose()
    }

    async fn upsert_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &UpsertMatterDocumentParams,
    ) -> Result<MatterDocumentRecord, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO matter_documents \
             (id, user_id, matter_id, memory_document_id, display_name, category, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
             ON CONFLICT (user_id, matter_id, memory_document_id) DO UPDATE SET \
                display_name = excluded.display_name, \
                category = excluded.category, \
                updated_at = datetime('now')",
            params![
                id.as_str(),
                user_id,
                matter_id,
                input.memory_document_id.to_string(),
                input.display_name.as_str(),
                input.category.as_str(),
            ],
        )
        .await?;

        // Prefer fetching by unique memory_document binding to cover both insert/update paths.
        let row = conn
            .query(
                "SELECT md.id, md.user_id, md.matter_id, md.memory_document_id, d.path, md.display_name, md.category, md.created_at, md.updated_at \
                 FROM matter_documents md \
                 JOIN memory_documents d ON d.id = md.memory_document_id \
                 WHERE md.user_id = ?1 AND md.matter_id = ?2 AND md.memory_document_id = ?3 LIMIT 1",
                params![user_id, matter_id, input.memory_document_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| {
                DatabaseError::Query("failed to resolve upserted matter document".to_string())
            })?;

        row_to_matter_document_record(&row)
    }

    async fn update_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
        input: &UpdateMatterDocumentParams,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError> {
        let Some(existing) = self
            .get_matter_document(user_id, matter_id, matter_document_id)
            .await?
        else {
            return Ok(None);
        };

        let merged_display_name = input.display_name.clone().unwrap_or(existing.display_name);
        let merged_category = input.category.unwrap_or(existing.category);

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE matter_documents SET \
                display_name = ?4, \
                category = ?5, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                matter_document_id.to_string(),
                merged_display_name,
                merged_category.as_str(),
            ],
        )
        .await?;

        self.get_matter_document(user_id, matter_id, matter_document_id)
            .await
    }

    async fn delete_matter_document(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM matter_documents WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, matter_document_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl DocumentVersionStore for LibSqlBackend {
    async fn list_document_versions(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Vec<DocumentVersionRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_document_id, version_number, label, memory_document_id, created_at, updated_at \
                 FROM document_versions \
                 WHERE user_id = ?1 AND matter_document_id = ?2 \
                 ORDER BY version_number DESC",
                params![user_id, matter_document_id.to_string()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_document_version_record(&row)?);
        }
        Ok(out)
    }

    async fn create_document_version(
        &self,
        user_id: &str,
        input: &CreateDocumentVersionParams,
    ) -> Result<DocumentVersionRecord, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;
        let version_result = async {
            let next_row = conn
                .query(
                    "SELECT COALESCE(MAX(version_number), 0) + 1 \
                     FROM document_versions \
                     WHERE user_id = ?1 AND matter_document_id = ?2",
                    params![user_id, input.matter_document_id.to_string()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| {
                    DatabaseError::Query("failed to compute next document version".to_string())
                })?;
            let next_version = i32::try_from(get_i64(&next_row, 0))
                .map_err(|_| DatabaseError::Serialization("invalid version number".to_string()))?;

            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO document_versions \
                 (id, user_id, matter_document_id, version_number, label, memory_document_id, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
                params![
                    id.as_str(),
                    user_id,
                    input.matter_document_id.to_string(),
                    i64::from(next_version),
                    input.label.as_str(),
                    input.memory_document_id.to_string(),
                ],
            )
            .await?;
            let row = conn
                .query(
                    "SELECT id, user_id, matter_document_id, version_number, label, memory_document_id, created_at, updated_at \
                     FROM document_versions \
                     WHERE id = ?1 LIMIT 1",
                    params![id.as_str()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| {
                    DatabaseError::Query("failed to load created document version".to_string())
                })?;
            row_to_document_version_record(&row)
        }
        .await;

        match version_result {
            Ok(record) => {
                conn.execute("COMMIT", ()).await?;
                Ok(record)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }
}

#[async_trait::async_trait]
impl DocumentTemplateStore for LibSqlBackend {
    async fn list_document_templates(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
    ) -> Result<Vec<DocumentTemplateRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = if let Some(matter_id) = matter_id {
            conn.query(
                "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                 FROM document_templates \
                 WHERE user_id = ?1 AND (matter_id = ?2 OR matter_id IS NULL) \
                 ORDER BY CASE WHEN matter_id = ?2 THEN 0 ELSE 1 END, name ASC",
                params![user_id, matter_id],
            )
            .await?
        } else {
            conn.query(
                "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                 FROM document_templates \
                 WHERE user_id = ?1 ORDER BY name ASC",
                params![user_id],
            )
            .await?
        };

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_document_template_record(&row)?);
        }
        Ok(out)
    }

    async fn get_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                 FROM document_templates \
                 WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, template_id.to_string()],
            )
            .await?
            .next()
            .await?;

        row.map(|row| row_to_document_template_record(&row))
            .transpose()
    }

    async fn get_document_template_by_name(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
        name: &str,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = if let Some(matter_id) = matter_id {
            conn.query(
                "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                 FROM document_templates \
                 WHERE user_id = ?1 AND name = ?2 AND (matter_id = ?3 OR matter_id IS NULL) \
                 ORDER BY CASE WHEN matter_id = ?3 THEN 0 ELSE 1 END \
                 LIMIT 1",
                params![user_id, name, matter_id],
            )
            .await?
            .next()
            .await?
        } else {
            conn.query(
                "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                 FROM document_templates \
                 WHERE user_id = ?1 AND name = ?2 AND matter_id IS NULL \
                 LIMIT 1",
                params![user_id, name],
            )
            .await?
            .next()
            .await?
        };

        row.map(|row| row_to_document_template_record(&row))
            .transpose()
    }

    async fn upsert_document_template(
        &self,
        user_id: &str,
        input: &UpsertDocumentTemplateParams,
    ) -> Result<DocumentTemplateRecord, DatabaseError> {
        let conn = self.connect().await?;
        let variables_json = if input.variables_json.is_object() || input.variables_json.is_array()
        {
            serde_json::to_string(&input.variables_json)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?
        } else {
            "[]".to_string()
        };

        conn.execute("BEGIN", ()).await?;
        let upsert_result = async {
            let row = if let Some(matter_id) = input.matter_id.as_deref() {
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO document_templates \
                     (id, user_id, matter_id, name, body, variables_json, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now')) \
                     ON CONFLICT (user_id, matter_id, name) DO UPDATE SET \
                        body = excluded.body, \
                        variables_json = excluded.variables_json, \
                        updated_at = datetime('now')",
                    params![
                        id.as_str(),
                        user_id,
                        matter_id,
                        input.name.as_str(),
                        input.body.as_str(),
                        variables_json.as_str(),
                    ],
                )
                .await?;
                conn.query(
                    "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                     FROM document_templates \
                     WHERE user_id = ?1 AND matter_id = ?2 AND name = ?3 LIMIT 1",
                    params![user_id, matter_id, input.name.as_str()],
                )
                .await?
                .next()
                .await?
            } else if let Some(existing) = conn
                .query(
                    "SELECT id FROM document_templates \
                     WHERE user_id = ?1 AND matter_id IS NULL AND name = ?2 LIMIT 1",
                    params![user_id, input.name.as_str()],
                )
                .await?
                .next()
                .await?
            {
                let existing_id = get_text(&existing, 0);
                conn.execute(
                    "UPDATE document_templates SET \
                        body = ?3, \
                        variables_json = ?4, \
                        updated_at = datetime('now') \
                     WHERE user_id = ?1 AND id = ?2",
                    params![
                        user_id,
                        existing_id.as_str(),
                        input.body.as_str(),
                        variables_json.as_str(),
                    ],
                )
                .await?;
                conn.query(
                    "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                     FROM document_templates \
                     WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                    params![user_id, existing_id.as_str()],
                )
                .await?
                .next()
                .await?
            } else {
                let id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO document_templates \
                     (id, user_id, matter_id, name, body, variables_json, created_at, updated_at) \
                     VALUES (?1, ?2, NULL, ?3, ?4, ?5, datetime('now'), datetime('now'))",
                    params![
                        id.as_str(),
                        user_id,
                        input.name.as_str(),
                        input.body.as_str(),
                        variables_json.as_str(),
                    ],
                )
                .await?;
                conn.query(
                    "SELECT id, user_id, matter_id, name, body, variables_json, created_at, updated_at \
                     FROM document_templates \
                     WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                    params![user_id, id.as_str()],
                )
                .await?
                .next()
                .await?
            };

            let row =
                row.ok_or_else(|| DatabaseError::Query("failed to load upserted template".to_string()))?;
            row_to_document_template_record(&row)
        }
        .await;

        match upsert_result {
            Ok(record) => {
                conn.execute("COMMIT", ()).await?;
                Ok(record)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn update_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
        input: &UpdateDocumentTemplateParams,
    ) -> Result<Option<DocumentTemplateRecord>, DatabaseError> {
        let Some(existing) = self.get_document_template(user_id, template_id).await? else {
            return Ok(None);
        };

        let merged_name = input.name.clone().unwrap_or(existing.name);
        let merged_body = input.body.clone().unwrap_or(existing.body);
        let merged_vars = input
            .variables_json
            .clone()
            .unwrap_or(existing.variables_json);
        let merged_vars = if merged_vars.is_object() || merged_vars.is_array() {
            serde_json::to_string(&merged_vars)
                .map_err(|e| DatabaseError::Serialization(e.to_string()))?
        } else {
            "[]".to_string()
        };

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE document_templates SET \
                name = ?3, \
                body = ?4, \
                variables_json = ?5, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                template_id.to_string(),
                merged_name,
                merged_body,
                merged_vars,
            ],
        )
        .await?;

        self.get_document_template(user_id, template_id).await
    }

    async fn delete_document_template(
        &self,
        user_id: &str,
        template_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM document_templates WHERE user_id = ?1 AND id = ?2",
                params![user_id, template_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }
}

#[async_trait::async_trait]
impl TimeExpenseStore for LibSqlBackend {
    async fn list_time_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<TimeEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, description, hours, hourly_rate, entry_date, billable, billed_invoice_id, created_at, updated_at \
                 FROM time_entries \
                 WHERE user_id = ?1 AND matter_id = ?2 \
                 ORDER BY entry_date DESC, created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_time_entry_record(&row)?);
        }
        Ok(out)
    }

    async fn get_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<Option<TimeEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, description, hours, hourly_rate, entry_date, billable, billed_invoice_id, created_at, updated_at \
                 FROM time_entries \
                 WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, entry_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_time_entry_record(&row)).transpose()
    }

    async fn create_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateTimeEntryParams,
    ) -> Result<TimeEntryRecord, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO time_entries \
             (id, user_id, matter_id, timekeeper, description, hours, hourly_rate, entry_date, billable, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, datetime('now'), datetime('now'))",
            params![
                id.as_str(),
                user_id,
                matter_id,
                input.timekeeper.as_str(),
                input.description.as_str(),
                input.hours.to_string(),
                opt_text_owned(input.hourly_rate.map(|rate| rate.to_string())),
                input.entry_date.format("%Y-%m-%d").to_string(),
                if input.billable { 1 } else { 0 },
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, description, hours, hourly_rate, entry_date, billable, billed_invoice_id, created_at, updated_at \
                 FROM time_entries WHERE id = ?1 LIMIT 1",
                params![id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load created time entry".to_string()))?;

        row_to_time_entry_record(&row)
    }

    async fn update_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
        input: &UpdateTimeEntryParams,
    ) -> Result<Option<TimeEntryRecord>, DatabaseError> {
        let Some(existing) = self.get_time_entry(user_id, matter_id, entry_id).await? else {
            return Ok(None);
        };

        let merged_timekeeper = input.timekeeper.clone().unwrap_or(existing.timekeeper);
        let merged_description = input.description.clone().unwrap_or(existing.description);
        let merged_hours = input.hours.unwrap_or(existing.hours);
        let merged_hourly_rate = input.hourly_rate.unwrap_or(existing.hourly_rate);
        let merged_entry_date = input.entry_date.unwrap_or(existing.entry_date);
        let merged_billable = input.billable.unwrap_or(existing.billable);

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE time_entries SET \
                timekeeper = ?4, \
                description = ?5, \
                hours = ?6, \
                hourly_rate = ?7, \
                entry_date = ?8, \
                billable = ?9, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                entry_id.to_string(),
                merged_timekeeper,
                merged_description,
                merged_hours.to_string(),
                opt_text_owned(merged_hourly_rate.map(|rate| rate.to_string())),
                merged_entry_date.format("%Y-%m-%d").to_string(),
                if merged_billable { 1 } else { 0 },
            ],
        )
        .await?;

        self.get_time_entry(user_id, matter_id, entry_id).await
    }

    async fn delete_time_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM time_entries WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, entry_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }

    async fn list_expense_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<ExpenseEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, submitted_by, description, amount, category, entry_date, receipt_path, billable, billed_invoice_id, created_at, updated_at \
                 FROM expense_entries \
                 WHERE user_id = ?1 AND matter_id = ?2 \
                 ORDER BY entry_date DESC, created_at DESC",
                params![user_id, matter_id],
            )
            .await?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_expense_entry_record(&row)?);
        }
        Ok(out)
    }

    async fn get_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<Option<ExpenseEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, submitted_by, description, amount, category, entry_date, receipt_path, billable, billed_invoice_id, created_at, updated_at \
                 FROM expense_entries \
                 WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3 LIMIT 1",
                params![user_id, matter_id, entry_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_expense_entry_record(&row)).transpose()
    }

    async fn create_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateExpenseEntryParams,
    ) -> Result<ExpenseEntryRecord, DatabaseError> {
        let conn = self.connect().await?;
        let id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO expense_entries \
             (id, user_id, matter_id, submitted_by, description, amount, category, entry_date, receipt_path, billable, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'), datetime('now'))",
            params![
                id.as_str(),
                user_id,
                matter_id,
                input.submitted_by.as_str(),
                input.description.as_str(),
                input.amount.to_string(),
                input.category.as_str(),
                input.entry_date.format("%Y-%m-%d").to_string(),
                opt_text(input.receipt_path.as_deref()),
                if input.billable { 1 } else { 0 },
            ],
        )
        .await?;

        let row = conn
            .query(
                "SELECT id, user_id, matter_id, submitted_by, description, amount, category, entry_date, receipt_path, billable, billed_invoice_id, created_at, updated_at \
                 FROM expense_entries WHERE id = ?1 LIMIT 1",
                params![id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| {
                DatabaseError::Query("failed to load created expense entry".to_string())
            })?;

        row_to_expense_entry_record(&row)
    }

    async fn update_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
        input: &UpdateExpenseEntryParams,
    ) -> Result<Option<ExpenseEntryRecord>, DatabaseError> {
        let Some(existing) = self.get_expense_entry(user_id, matter_id, entry_id).await? else {
            return Ok(None);
        };

        let merged_submitted_by = input.submitted_by.clone().unwrap_or(existing.submitted_by);
        let merged_description = input.description.clone().unwrap_or(existing.description);
        let merged_amount = input.amount.unwrap_or(existing.amount);
        let merged_category = input.category.unwrap_or(existing.category);
        let merged_entry_date = input.entry_date.unwrap_or(existing.entry_date);
        let merged_receipt_path = input.receipt_path.clone().unwrap_or(existing.receipt_path);
        let merged_billable = input.billable.unwrap_or(existing.billable);

        let conn = self.connect().await?;
        conn.execute(
            "UPDATE expense_entries SET \
                submitted_by = ?4, \
                description = ?5, \
                amount = ?6, \
                category = ?7, \
                entry_date = ?8, \
                receipt_path = ?9, \
                billable = ?10, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                entry_id.to_string(),
                merged_submitted_by,
                merged_description,
                merged_amount.to_string(),
                merged_category.as_str(),
                merged_entry_date.format("%Y-%m-%d").to_string(),
                opt_text(merged_receipt_path.as_deref()),
                if merged_billable { 1 } else { 0 },
            ],
        )
        .await?;

        self.get_expense_entry(user_id, matter_id, entry_id).await
    }

    async fn delete_expense_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        entry_id: Uuid,
    ) -> Result<bool, DatabaseError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM expense_entries WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
                params![user_id, matter_id, entry_id.to_string()],
            )
            .await?;
        Ok(deleted > 0)
    }

    async fn mark_time_entries_billed(
        &self,
        user_id: &str,
        entry_ids: &[Uuid],
        invoice_id: &str,
    ) -> Result<u64, DatabaseError> {
        if entry_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;
        let mark_result = async {
            let mut count: u64 = 0;
            for entry_id in entry_ids {
                let updated = conn
                    .execute(
                        "UPDATE time_entries SET \
                            billed_invoice_id = ?3, \
                            updated_at = datetime('now') \
                         WHERE user_id = ?1 AND id = ?2 AND billed_invoice_id IS NULL",
                        params![user_id, entry_id.to_string(), invoice_id],
                    )
                    .await?;
                count += updated;
            }
            Ok::<u64, DatabaseError>(count)
        }
        .await;

        match mark_result {
            Ok(count) => {
                conn.execute("COMMIT", ()).await?;
                Ok(count)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn mark_expense_entries_billed(
        &self,
        user_id: &str,
        entry_ids: &[Uuid],
        invoice_id: &str,
    ) -> Result<u64, DatabaseError> {
        if entry_ids.is_empty() {
            return Ok(0);
        }
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;
        let mark_result = async {
            let mut count: u64 = 0;
            for entry_id in entry_ids {
                let updated = conn
                    .execute(
                        "UPDATE expense_entries SET \
                            billed_invoice_id = ?3, \
                            updated_at = datetime('now') \
                         WHERE user_id = ?1 AND id = ?2 AND billed_invoice_id IS NULL",
                        params![user_id, entry_id.to_string(), invoice_id],
                    )
                    .await?;
                count += updated;
            }
            Ok::<u64, DatabaseError>(count)
        }
        .await;

        match mark_result {
            Ok(count) => {
                conn.execute("COMMIT", ()).await?;
                Ok(count)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn matter_time_summary(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<MatterTimeSummary, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(hours AS REAL)) FROM time_entries WHERE user_id = ?1 AND matter_id = ?2), 0.0)) AS total_hours, \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(hours AS REAL)) FROM time_entries WHERE user_id = ?1 AND matter_id = ?2 AND billable = 1), 0.0)) AS billable_hours, \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(hours AS REAL)) FROM time_entries WHERE user_id = ?1 AND matter_id = ?2 AND billed_invoice_id IS NULL), 0.0)) AS unbilled_hours, \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(amount AS REAL)) FROM expense_entries WHERE user_id = ?1 AND matter_id = ?2), 0.0)) AS total_expenses, \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(amount AS REAL)) FROM expense_entries WHERE user_id = ?1 AND matter_id = ?2 AND billable = 1), 0.0)) AS billable_expenses, \
                    printf('%.2f', COALESCE((SELECT SUM(CAST(amount AS REAL)) FROM expense_entries WHERE user_id = ?1 AND matter_id = ?2 AND billed_invoice_id IS NULL), 0.0)) AS unbilled_expenses",
                params![user_id, matter_id],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to read time summary".to_string()))?;

        let parse_dec = |idx| -> Result<Decimal, DatabaseError> {
            let raw = get_text(&row, idx);
            raw.parse::<Decimal>().map_err(|_| {
                DatabaseError::Serialization(format!(
                    "failed to parse decimal summary value '{}'",
                    raw
                ))
            })
        };

        Ok(MatterTimeSummary {
            total_hours: parse_dec(0)?,
            billable_hours: parse_dec(1)?,
            unbilled_hours: parse_dec(2)?,
            total_expenses: parse_dec(3)?,
            billable_expenses: parse_dec(4)?,
            unbilled_expenses: parse_dec(5)?,
        })
    }
}

#[async_trait::async_trait]
impl BillingStore for LibSqlBackend {
    async fn save_invoice_draft(
        &self,
        user_id: &str,
        invoice: &CreateInvoiceParams,
        line_items: &[CreateInvoiceLineItemParams],
    ) -> Result<(InvoiceRecord, Vec<InvoiceLineItemRecord>), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;
        let result = async {
            let invoice_id = Uuid::new_v4().to_string();
            let issued_date = invoice
                .issued_date
                .map(|value| value.format("%Y-%m-%d").to_string());
            let due_date = invoice
                .due_date
                .map(|value| value.format("%Y-%m-%d").to_string());
            conn.execute(
                "INSERT INTO invoices (id, user_id, matter_id, invoice_number, status, issued_date, due_date, subtotal, tax, total, paid_amount, notes, created_at, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, datetime('now'), datetime('now'))",
                params![
                    invoice_id.as_str(),
                    user_id,
                    invoice.matter_id.as_str(),
                    invoice.invoice_number.as_str(),
                    invoice.status.as_str(),
                    opt_text_owned(issued_date),
                    opt_text_owned(due_date),
                    invoice.subtotal.to_string(),
                    invoice.tax.to_string(),
                    invoice.total.to_string(),
                    invoice.paid_amount.to_string(),
                    opt_text(invoice.notes.as_deref()),
                ],
            )
            .await?;
            let invoice_row = conn
                .query(
                    "SELECT id, user_id, matter_id, invoice_number, status, issued_date, due_date, subtotal, tax, total, paid_amount, notes, created_at, updated_at \
                     FROM invoices WHERE id = ?1 LIMIT 1",
                    params![invoice_id.as_str()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| DatabaseError::Query("failed to load created invoice".to_string()))?;
            let invoice_record = row_to_invoice_record(&invoice_row)?;

            let mut persisted_items = Vec::with_capacity(line_items.len());
            for item in line_items {
                let line_item_id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO invoice_line_items (id, user_id, invoice_id, description, quantity, unit_price, amount, time_entry_id, expense_entry_id, sort_order, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, datetime('now'), datetime('now'))",
                    params![
                        line_item_id.as_str(),
                        user_id,
                        invoice_id.as_str(),
                        item.description.as_str(),
                        item.quantity.to_string(),
                        item.unit_price.to_string(),
                        item.amount.to_string(),
                        opt_text_owned(item.time_entry_id.map(|value| value.to_string())),
                        opt_text_owned(item.expense_entry_id.map(|value| value.to_string())),
                        item.sort_order,
                    ],
                )
                .await?;
                let row = conn
                    .query(
                        "SELECT id, user_id, invoice_id, description, quantity, unit_price, amount, time_entry_id, expense_entry_id, sort_order, created_at, updated_at \
                         FROM invoice_line_items WHERE id = ?1 LIMIT 1",
                        params![line_item_id.as_str()],
                    )
                    .await?
                    .next()
                    .await?
                    .ok_or_else(|| {
                        DatabaseError::Query("failed to load created invoice line item".to_string())
                    })?;
                persisted_items.push(row_to_invoice_line_item_record(&row)?);
            }

            Ok::<(InvoiceRecord, Vec<InvoiceLineItemRecord>), DatabaseError>((
                invoice_record,
                persisted_items,
            ))
        }
        .await;

        match result {
            Ok(result) => {
                conn.execute("COMMIT", ()).await?;
                Ok(result)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn get_invoice(
        &self,
        user_id: &str,
        invoice_id: Uuid,
    ) -> Result<Option<InvoiceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, invoice_number, status, issued_date, due_date, subtotal, tax, total, paid_amount, notes, created_at, updated_at \
                 FROM invoices WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, invoice_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_invoice_record(&row)).transpose()
    }

    async fn list_invoice_line_items(
        &self,
        user_id: &str,
        invoice_id: Uuid,
    ) -> Result<Vec<InvoiceLineItemRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, invoice_id, description, quantity, unit_price, amount, time_entry_id, expense_entry_id, sort_order, created_at, updated_at \
                 FROM invoice_line_items \
                 WHERE user_id = ?1 AND invoice_id = ?2 \
                 ORDER BY sort_order ASC, created_at ASC",
                params![user_id, invoice_id.to_string()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_invoice_line_item_record(&row)?);
        }
        Ok(out)
    }

    async fn set_invoice_status(
        &self,
        user_id: &str,
        invoice_id: Uuid,
        status: InvoiceStatus,
        issued_date: Option<NaiveDate>,
    ) -> Result<Option<InvoiceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let issued_date = issued_date.map(|value| value.format("%Y-%m-%d").to_string());
        conn.execute(
            "UPDATE invoices SET \
                status = ?3, \
                issued_date = COALESCE(?4, issued_date), \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                invoice_id.to_string(),
                status.as_str(),
                opt_text_owned(issued_date),
            ],
        )
        .await?;
        self.get_invoice(user_id, invoice_id).await
    }

    async fn apply_invoice_payment(
        &self,
        user_id: &str,
        invoice_id: Uuid,
        amount: Decimal,
    ) -> Result<Option<InvoiceRecord>, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE invoices SET \
                paid_amount = printf('%.2f', CAST(paid_amount AS REAL) + CAST(?3 AS REAL)), \
                status = CASE \
                    WHEN CAST(paid_amount AS REAL) + CAST(?3 AS REAL) >= CAST(total AS REAL) THEN 'paid' \
                    ELSE status \
                END, \
                updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![user_id, invoice_id.to_string(), amount.to_string()],
        )
        .await?;
        self.get_invoice(user_id, invoice_id).await
    }

    async fn append_trust_ledger_entry(
        &self,
        user_id: &str,
        matter_id: &str,
        input: &CreateTrustLedgerEntryParams,
    ) -> Result<TrustLedgerEntryRecord, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let result = async {
            let balance_row = conn
                .query(
                    "SELECT COALESCE((SELECT balance_after FROM trust_ledger WHERE user_id = ?1 AND matter_id = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1), '0') AS balance",
                    params![user_id, matter_id],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| DatabaseError::Query("failed to read trust balance".to_string()))?;
            let current_balance_raw = get_text(&balance_row, 0);
            let current_balance = current_balance_raw.parse::<Decimal>().map_err(|_| {
                DatabaseError::Serialization(format!(
                    "failed to parse trust balance '{}'",
                    current_balance_raw
                ))
            })?;
            let balance_after = (current_balance + input.delta).round_dp(2);
            if balance_after < Decimal::ZERO {
                return Err(DatabaseError::Constraint(
                    "insufficient trust balance for requested entry".to_string(),
                ));
            }

            let entry_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO trust_ledger (id, user_id, matter_id, entry_type, amount, balance_after, description, invoice_id, recorded_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                params![
                    entry_id.as_str(),
                    user_id,
                    matter_id,
                    input.entry_type.as_str(),
                    input.amount.to_string(),
                    balance_after.to_string(),
                    input.description.as_str(),
                    opt_text_owned(input.invoice_id.map(|value| value.to_string())),
                    input.recorded_by.as_str(),
                ],
            )
            .await?;
            let row = conn
                .query(
                    "SELECT id, user_id, matter_id, entry_type, amount, balance_after, description, invoice_id, recorded_by, created_at \
                     FROM trust_ledger WHERE id = ?1 LIMIT 1",
                    params![entry_id.as_str()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| {
                    DatabaseError::Query("failed to load created trust ledger entry".to_string())
                })?;
            row_to_trust_ledger_entry_record(&row)
        }
        .await;

        match result {
            Ok(record) => {
                conn.execute("COMMIT", ()).await?;
                Ok(record)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn list_trust_ledger_entries(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Vec<TrustLedgerEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, entry_type, amount, balance_after, description, invoice_id, recorded_by, created_at \
                 FROM trust_ledger WHERE user_id = ?1 AND matter_id = ?2 ORDER BY created_at DESC, rowid DESC",
                params![user_id, matter_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_trust_ledger_entry_record(&row)?);
        }
        Ok(out)
    }

    async fn current_trust_balance(
        &self,
        user_id: &str,
        matter_id: &str,
    ) -> Result<Decimal, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT COALESCE((SELECT balance_after FROM trust_ledger WHERE user_id = ?1 AND matter_id = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1), '0') AS balance",
                params![user_id, matter_id],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to read trust balance".to_string()))?;
        let raw = get_text(&row, 0);
        raw.parse::<Decimal>().map_err(|_| {
            DatabaseError::Serialization(format!("failed to parse trust balance '{}'", raw))
        })
    }
}
