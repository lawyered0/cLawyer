use chrono::Utc;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::{
    BillingRateScheduleRecord, BillingRateStore, CitationVerificationResultRecord,
    CitationVerificationRunRecord, CitationVerificationStatus, CitationVerificationStore,
    ComputeTrustReconciliationParams, CreateBillingRateScheduleParams,
    CreateCitationVerificationResultParams, CreateCitationVerificationRunParams,
    CreateTrustStatementImportParams, CreateTrustStatementLineParams, DocumentReadinessState,
    MatterDocumentRecord, MatterDocumentStore, TrustAccountRecord, TrustAccountingStore,
    TrustLedgerEntryRecord, TrustReconciliationRecord, TrustReconciliationStatus,
    TrustStatementImportRecord, TrustStatementLineRecord, UpdateBillingRateScheduleParams,
    UpsertTrustAccountParams,
};
use crate::error::DatabaseError;

use super::PgBackend;

fn row_to_billing_rate_schedule(
    row: &tokio_postgres::Row,
) -> Result<BillingRateScheduleRecord, DatabaseError> {
    Ok(BillingRateScheduleRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        timekeeper: row.get("timekeeper"),
        rate: row.get("rate"),
        effective_start: row.get("effective_start"),
        effective_end: row.get("effective_end"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_citation_run(
    row: &tokio_postgres::Row,
) -> Result<CitationVerificationRunRecord, DatabaseError> {
    Ok(CitationVerificationRunRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        matter_document_id: row.get("matter_document_id"),
        provider: row.get("provider"),
        document_hash: row.get("document_hash"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

fn row_to_citation_result(
    row: &tokio_postgres::Row,
) -> Result<CitationVerificationResultRecord, DatabaseError> {
    let status_raw: String = row.get("status");
    let status = CitationVerificationStatus::from_db_value(&status_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!(
            "invalid citation verification status '{}'",
            status_raw
        ))
    })?;
    Ok(CitationVerificationResultRecord {
        id: row.get("id"),
        run_id: row.get("run_id"),
        citation_text: row.get("citation_text"),
        normalized_citation: row.get("normalized_citation"),
        status,
        provider_reference: row.get("provider_reference"),
        provider_title: row.get("provider_title"),
        detail: row.get("detail"),
        waived_by: row.get("waived_by"),
        waiver_reason: row.get("waiver_reason"),
        waived_at: row.get("waived_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_trust_account(row: &tokio_postgres::Row) -> Result<TrustAccountRecord, DatabaseError> {
    Ok(TrustAccountRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        bank_name: row.get("bank_name"),
        account_number_last4: row.get("account_number_last4"),
        is_primary: row.get("is_primary"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_trust_statement_import(
    row: &tokio_postgres::Row,
) -> Result<TrustStatementImportRecord, DatabaseError> {
    Ok(TrustStatementImportRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        trust_account_id: row.get("trust_account_id"),
        statement_date: row.get("statement_date"),
        starting_balance: row.get("starting_balance"),
        ending_balance: row.get("ending_balance"),
        imported_by: row.get("imported_by"),
        row_count: row.get("row_count"),
        created_at: row.get("created_at"),
    })
}

fn row_to_trust_statement_line(
    row: &tokio_postgres::Row,
) -> Result<TrustStatementLineRecord, DatabaseError> {
    Ok(TrustStatementLineRecord {
        id: row.get("id"),
        statement_import_id: row.get("statement_import_id"),
        entry_date: row.get("entry_date"),
        description: row.get("description"),
        debit: row.get("debit"),
        credit: row.get("credit"),
        running_balance: row.get("running_balance"),
        reference_number: row.get("reference_number"),
        created_at: row.get("created_at"),
    })
}

fn row_to_trust_reconciliation(
    row: &tokio_postgres::Row,
) -> Result<TrustReconciliationRecord, DatabaseError> {
    let status_raw: String = row.get("status");
    let status = TrustReconciliationStatus::from_db_value(&status_raw).ok_or_else(|| {
        DatabaseError::Serialization(format!(
            "invalid trust reconciliation status '{}'",
            status_raw
        ))
    })?;
    Ok(TrustReconciliationRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        trust_account_id: row.get("trust_account_id"),
        statement_import_id: row.get("statement_import_id"),
        statement_ending_balance: row.get("statement_ending_balance"),
        book_balance: row.get("book_balance"),
        client_balance_total: row.get("client_balance_total"),
        difference: row.get("difference"),
        exceptions_json: row.get("exceptions_json"),
        status,
        signed_off_by: row.get("signed_off_by"),
        signed_off_at: row.get("signed_off_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn row_to_trust_ledger_entry(
    row: &tokio_postgres::Row,
) -> Result<TrustLedgerEntryRecord, DatabaseError> {
    let entry_type_raw: String = row.get("entry_type");
    let entry_detail: Option<String> = row.get("entry_detail");
    let source_raw: Option<String> = row.get("source");
    let entry_type =
        crate::db::TrustLedgerEntryType::from_db_columns(&entry_type_raw, entry_detail.as_deref())
            .ok_or_else(|| {
                DatabaseError::Serialization(format!(
                    "invalid trust ledger entry type '{}' / detail {:?}",
                    entry_type_raw, entry_detail
                ))
            })?;
    let source = source_raw
        .as_deref()
        .and_then(crate::db::TrustLedgerSource::from_db_value)
        .unwrap_or_default();
    Ok(TrustLedgerEntryRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        matter_id: row.get("matter_id"),
        trust_account_id: row.get("trust_account_id"),
        entry_type,
        amount: row.get("amount"),
        delta: row.get("delta"),
        balance_after: row.get("balance_after"),
        description: row.get("description"),
        reference_number: row.get("reference_number"),
        source,
        invoice_id: row.get("invoice_id"),
        recorded_by: row.get("recorded_by"),
        created_at: row.get("created_at"),
    })
}

#[async_trait::async_trait]
impl BillingRateStore for PgBackend {
    async fn list_billing_rate_schedules(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
        timekeeper: Option<&str>,
    ) -> Result<Vec<BillingRateScheduleRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules \
                 WHERE user_id = $1 \
                   AND ($2::text IS NULL OR matter_id = $2) \
                   AND ($3::text IS NULL OR timekeeper = $3) \
                 ORDER BY COALESCE(matter_id, ''), timekeeper, effective_start DESC",
                &[&user_id, &matter_id, &timekeeper],
            )
            .await?;
        rows.iter().map(row_to_billing_rate_schedule).collect()
    }

    async fn create_billing_rate_schedule(
        &self,
        user_id: &str,
        input: &CreateBillingRateScheduleParams,
    ) -> Result<BillingRateScheduleRecord, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO billing_rate_schedules \
                 (id, user_id, matter_id, timekeeper, rate, effective_start, effective_end) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 RETURNING id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at",
                &[
                    &Uuid::new_v4(),
                    &user_id,
                    &input.matter_id,
                    &input.timekeeper,
                    &input.rate,
                    &input.effective_start,
                    &input.effective_end,
                ],
            )
            .await?;
        row_to_billing_rate_schedule(&row)
    }

    async fn update_billing_rate_schedule(
        &self,
        user_id: &str,
        schedule_id: Uuid,
        input: &UpdateBillingRateScheduleParams,
    ) -> Result<Option<BillingRateScheduleRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let existing = conn
            .query_opt(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules WHERE user_id = $1 AND id = $2 LIMIT 1",
                &[&user_id, &schedule_id],
            )
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let existing = row_to_billing_rate_schedule(&existing)?;
        let row = conn
            .query_one(
                "UPDATE billing_rate_schedules SET \
                    matter_id = $3, timekeeper = $4, rate = $5, effective_start = $6, effective_end = $7, updated_at = NOW() \
                 WHERE user_id = $1 AND id = $2 \
                 RETURNING id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at",
                &[
                    &user_id,
                    &schedule_id,
                    &input.matter_id.clone().unwrap_or(existing.matter_id),
                    &input.timekeeper.clone().unwrap_or(existing.timekeeper),
                    &input.rate.unwrap_or(existing.rate),
                    &input.effective_start.unwrap_or(existing.effective_start),
                    &input.effective_end.unwrap_or(existing.effective_end),
                ],
            )
            .await?;
        Ok(Some(row_to_billing_rate_schedule(&row)?))
    }
}

#[async_trait::async_trait]
impl CitationVerificationStore for PgBackend {
    async fn create_citation_verification_run(
        &self,
        user_id: &str,
        input: &CreateCitationVerificationRunParams,
        results: &[CreateCitationVerificationResultParams],
    ) -> Result<
        (
            CitationVerificationRunRecord,
            Vec<CitationVerificationResultRecord>,
        ),
        DatabaseError,
    > {
        let mut conn = self.store.conn().await?;
        let tx = conn.transaction().await?;
        let run_id = Uuid::new_v4();
        let run_row = tx
            .query_one(
                "INSERT INTO citation_verification_runs \
                 (id, user_id, matter_id, matter_document_id, provider, document_hash, created_by) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7) \
                 RETURNING id, user_id, matter_id, matter_document_id, provider, document_hash, created_by, created_at",
                &[
                    &run_id,
                    &user_id,
                    &input.matter_id,
                    &input.matter_document_id,
                    &input.provider,
                    &input.document_hash,
                    &input.created_by,
                ],
            )
            .await?;
        let run = row_to_citation_run(&run_row)?;
        let mut stored_results = Vec::new();
        for result in results {
            let row = tx
                .query_one(
                    "INSERT INTO citation_verification_results \
                     (id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
                     RETURNING id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at, created_at, updated_at",
                    &[
                        &Uuid::new_v4(),
                        &run_id,
                        &result.citation_text,
                        &result.normalized_citation,
                        &result.status.as_str(),
                        &result.provider_reference,
                        &result.provider_title,
                        &result.detail,
                        &result.waived_by,
                        &result.waiver_reason,
                        &result.waived_at,
                    ],
                )
                .await?;
            stored_results.push(row_to_citation_result(&row)?);
        }
        let readiness_state = if stored_results.is_empty() {
            DocumentReadinessState::Draft
        } else if stored_results
            .iter()
            .all(|result| matches!(result.status, CitationVerificationStatus::Waived))
        {
            DocumentReadinessState::Waived
        } else if stored_results.iter().all(|result| {
            matches!(
                result.status,
                CitationVerificationStatus::Verified | CitationVerificationStatus::Waived
            )
        }) {
            DocumentReadinessState::Verified
        } else {
            DocumentReadinessState::CitationsPending
        };
        tx.execute(
            "UPDATE matter_documents SET readiness_state = $3, updated_at = NOW() \
             WHERE user_id = $1 AND id = $2",
            &[
                &user_id,
                &input.matter_document_id,
                &readiness_state.as_str(),
            ],
        )
        .await?;
        tx.commit().await?;
        Ok((run, stored_results))
    }

    async fn latest_citation_verification_run(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<CitationVerificationRunRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, matter_id, matter_document_id, provider, document_hash, created_by, created_at \
                 FROM citation_verification_runs \
                 WHERE user_id = $1 AND matter_document_id = $2 \
                 ORDER BY created_at DESC LIMIT 1",
                &[&user_id, &matter_document_id],
            )
            .await?;
        row.map(|row| row_to_citation_run(&row)).transpose()
    }

    async fn list_citation_verification_results(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Vec<CitationVerificationResultRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at, created_at, updated_at \
                 FROM citation_verification_results \
                 WHERE run_id = ( \
                    SELECT id FROM citation_verification_runs \
                    WHERE user_id = $1 AND matter_document_id = $2 \
                    ORDER BY created_at DESC LIMIT 1 \
                 ) \
                 ORDER BY created_at ASC",
                &[&user_id, &matter_document_id],
            )
            .await?;
        rows.iter().map(row_to_citation_result).collect()
    }

    async fn set_matter_document_readiness(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
        state: DocumentReadinessState,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        conn.execute(
            "UPDATE matter_documents SET readiness_state = $4, updated_at = NOW() \
             WHERE user_id = $1 AND matter_id = $2 AND id = $3",
            &[&user_id, &matter_id, &matter_document_id, &state.as_str()],
        )
        .await?;
        self.get_matter_document(user_id, matter_id, matter_document_id)
            .await
    }
}

#[async_trait::async_trait]
impl TrustAccountingStore for PgBackend {
    async fn get_primary_trust_account(
        &self,
        user_id: &str,
    ) -> Result<Option<TrustAccountRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at \
                 FROM trust_accounts WHERE user_id = $1 AND is_primary = TRUE LIMIT 1",
                &[&user_id],
            )
            .await?;
        row.map(|row| row_to_trust_account(&row)).transpose()
    }

    async fn upsert_primary_trust_account(
        &self,
        user_id: &str,
        input: &UpsertTrustAccountParams,
    ) -> Result<TrustAccountRecord, DatabaseError> {
        let conn = self.store.conn().await?;
        if let Some(existing) = self.get_primary_trust_account(user_id).await? {
            let row = conn
                .query_one(
                    "UPDATE trust_accounts SET name = $3, bank_name = $4, account_number_last4 = $5, updated_at = NOW() \
                     WHERE user_id = $1 AND id = $2 \
                     RETURNING id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at",
                    &[&user_id, &existing.id, &input.name, &input.bank_name, &input.account_number_last4],
                )
                .await?;
            return row_to_trust_account(&row);
        }
        let row = conn
            .query_one(
                "INSERT INTO trust_accounts \
                 (id, user_id, name, bank_name, account_number_last4, is_primary) \
                 VALUES ($1, $2, $3, $4, $5, TRUE) \
                 RETURNING id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at",
                &[&Uuid::new_v4(), &user_id, &input.name, &input.bank_name, &input.account_number_last4],
            )
            .await?;
        row_to_trust_account(&row)
    }

    async fn list_trust_ledger_entries_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Vec<TrustLedgerEntryRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT id, user_id, matter_id, entry_type, amount, balance_after, description, invoice_id, recorded_by, created_at, trust_account_id, entry_detail, delta, reference_number, source \
                 FROM trust_ledger \
                 WHERE user_id = $1 AND trust_account_id = $2 \
                 ORDER BY created_at DESC, id DESC",
                &[&user_id, &trust_account_id],
            )
            .await?;
        rows.iter().map(row_to_trust_ledger_entry).collect()
    }

    async fn current_trust_account_balance(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Decimal, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "SELECT COALESCE((SELECT balance_after FROM trust_ledger WHERE user_id = $1 AND trust_account_id = $2 ORDER BY created_at DESC, id DESC LIMIT 1), 0)::numeric AS balance",
                &[&user_id, &trust_account_id],
            )
            .await?;
        Ok(row.get("balance"))
    }

    async fn import_trust_statement(
        &self,
        user_id: &str,
        input: &CreateTrustStatementImportParams,
        lines: &[CreateTrustStatementLineParams],
    ) -> Result<(TrustStatementImportRecord, Vec<TrustStatementLineRecord>), DatabaseError> {
        let mut conn = self.store.conn().await?;
        let tx = conn.transaction().await?;
        let import_id = Uuid::new_v4();
        let import_row = tx
            .query_one(
                "INSERT INTO trust_statement_imports \
                 (id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 RETURNING id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count, created_at",
                &[&import_id, &user_id, &input.trust_account_id, &input.statement_date, &input.starting_balance, &input.ending_balance, &input.imported_by, &(lines.len() as i32)],
            )
            .await?;
        let import = row_to_trust_statement_import(&import_row)?;
        let mut stored_lines = Vec::new();
        for line in lines {
            let row = tx
                .query_one(
                    "INSERT INTO trust_statement_lines \
                     (id, statement_import_id, entry_date, description, debit, credit, running_balance, reference_number) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     RETURNING id, statement_import_id, entry_date, description, debit, credit, running_balance, reference_number, created_at",
                    &[&Uuid::new_v4(), &import_id, &line.entry_date, &line.description, &line.debit, &line.credit, &line.running_balance, &line.reference_number],
                )
                .await?;
            stored_lines.push(row_to_trust_statement_line(&row)?);
        }
        tx.commit().await?;
        Ok((import, stored_lines))
    }

    async fn get_trust_statement_import(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Option<TrustStatementImportRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count, created_at \
                 FROM trust_statement_imports WHERE user_id = $1 AND id = $2 LIMIT 1",
                &[&user_id, &statement_import_id],
            )
            .await?;
        row.map(|row| row_to_trust_statement_import(&row))
            .transpose()
    }

    async fn list_trust_statement_lines(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Vec<TrustStatementLineRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let rows = conn
            .query(
                "SELECT tsl.id, tsl.statement_import_id, tsl.entry_date, tsl.description, tsl.debit, tsl.credit, tsl.running_balance, tsl.reference_number, tsl.created_at \
                 FROM trust_statement_lines tsl \
                 JOIN trust_statement_imports tsi ON tsi.id = tsl.statement_import_id \
                 WHERE tsi.user_id = $1 AND tsl.statement_import_id = $2 \
                 ORDER BY tsl.entry_date ASC, tsl.created_at ASC",
                &[&user_id, &statement_import_id],
            )
            .await?;
        rows.iter().map(row_to_trust_statement_line).collect()
    }

    async fn compute_trust_reconciliation(
        &self,
        user_id: &str,
        input: &ComputeTrustReconciliationParams,
    ) -> Result<TrustReconciliationRecord, DatabaseError> {
        let statement_import = self
            .get_trust_statement_import(user_id, input.statement_import_id)
            .await?
            .ok_or_else(|| DatabaseError::Query("trust statement import not found".to_string()))?;
        let statement_lines = self
            .list_trust_statement_lines(user_id, input.statement_import_id)
            .await?;
        let ledger_entries = self
            .list_trust_ledger_entries_for_account(user_id, input.trust_account_id)
            .await?;
        let book_balance = self
            .current_trust_account_balance(user_id, input.trust_account_id)
            .await?;

        let mut matter_balances: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        for entry in &ledger_entries {
            *matter_balances
                .entry(entry.matter_id.clone())
                .or_insert(Decimal::ZERO) += entry.delta;
        }
        let client_balance_total = matter_balances.values().copied().sum::<Decimal>();

        let mut exceptions = Vec::new();
        for line in &statement_lines {
            let target_delta = line.credit - line.debit;
            let matched = ledger_entries.iter().any(|entry| {
                entry.delta.round_dp(2) == target_delta.round_dp(2)
                    && entry.reference_number == line.reference_number
            });
            if !matched {
                exceptions.push(serde_json::json!({
                    "kind": "statement_unmatched",
                    "date": line.entry_date.to_string(),
                    "description": line.description,
                    "reference": line.reference_number,
                    "delta": target_delta.to_string(),
                }));
            }
        }

        let difference = (statement_import.ending_balance - book_balance).round_dp(2);
        let exceptions_json = serde_json::Value::Array(exceptions);
        let conn = self.store.conn().await?;
        let row = conn
            .query_one(
                "INSERT INTO trust_reconciliations \
                 (id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'draft', $9, NULL, NULL) \
                 RETURNING id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at",
                &[&Uuid::new_v4(), &user_id, &input.trust_account_id, &input.statement_import_id, &statement_import.ending_balance, &book_balance, &client_balance_total, &exceptions_json, &difference],
            )
            .await?;
        row_to_trust_reconciliation(&row)
    }

    async fn get_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at \
                 FROM trust_reconciliations WHERE user_id = $1 AND id = $2 LIMIT 1",
                &[&user_id, &reconciliation_id],
            )
            .await?;
        row.map(|row| row_to_trust_reconciliation(&row)).transpose()
    }

    async fn latest_trust_reconciliation_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        let row = conn
            .query_opt(
                "SELECT id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at \
                 FROM trust_reconciliations \
                 WHERE user_id = $1 AND trust_account_id = $2 \
                 ORDER BY created_at DESC LIMIT 1",
                &[&user_id, &trust_account_id],
            )
            .await?;
        row.map(|row| row_to_trust_reconciliation(&row)).transpose()
    }

    async fn signoff_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
        signed_off_by: &str,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.store.conn().await?;
        conn.execute(
            "UPDATE trust_reconciliations SET status = 'signed_off', signed_off_by = $3, signed_off_at = $4, updated_at = NOW() \
             WHERE user_id = $1 AND id = $2",
            &[&user_id, &reconciliation_id, &signed_off_by, &Utc::now()],
        )
        .await?;
        self.get_trust_reconciliation(user_id, reconciliation_id)
            .await
    }
}
