use chrono::{NaiveDate, Utc};
use libsql::params;
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

use super::{
    LibSqlBackend, fmt_ts, get_decimal, get_i64, get_opt_text, get_text, opt_text, opt_text_owned,
    parse_timestamp,
};

fn parse_uuid(raw: &str, field: &str) -> Result<Uuid, DatabaseError> {
    Uuid::parse_str(raw)
        .map_err(|e| DatabaseError::Serialization(format!("invalid {field} uuid: {e}")))
}

fn parse_naive_date(raw: &str, field: &str) -> Result<NaiveDate, DatabaseError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d").map_err(|_| {
        DatabaseError::Serialization(format!("invalid {field} date '{raw}'; expected YYYY-MM-DD"))
    })
}

fn parse_citation_status(raw: &str) -> Result<CitationVerificationStatus, DatabaseError> {
    CitationVerificationStatus::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid citation verification status '{raw}'"))
    })
}

fn parse_trust_reconciliation_status(
    raw: &str,
) -> Result<TrustReconciliationStatus, DatabaseError> {
    TrustReconciliationStatus::from_db_value(raw).ok_or_else(|| {
        DatabaseError::Serialization(format!("invalid trust reconciliation status '{raw}'"))
    })
}

fn row_to_billing_rate_schedule(
    row: &libsql::Row,
) -> Result<BillingRateScheduleRecord, DatabaseError> {
    Ok(BillingRateScheduleRecord {
        id: parse_uuid(&get_text(row, 0), "billing_rate_schedules.id")?,
        user_id: get_text(row, 1),
        matter_id: get_opt_text(row, 2),
        timekeeper: get_text(row, 3),
        rate: get_decimal(row, 4),
        effective_start: parse_naive_date(&get_text(row, 5), "effective_start")?,
        effective_end: get_opt_text(row, 6)
            .map(|value| parse_naive_date(&value, "effective_end"))
            .transpose()?,
        created_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 8))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_citation_run(row: &libsql::Row) -> Result<CitationVerificationRunRecord, DatabaseError> {
    Ok(CitationVerificationRunRecord {
        id: parse_uuid(&get_text(row, 0), "citation_verification_runs.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        matter_document_id: parse_uuid(
            &get_text(row, 3),
            "citation_verification_runs.matter_document_id",
        )?,
        provider: get_text(row, 4),
        document_hash: get_text(row, 5),
        created_by: get_text(row, 6),
        created_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_citation_result(
    row: &libsql::Row,
) -> Result<CitationVerificationResultRecord, DatabaseError> {
    let status_raw = get_text(row, 4);
    Ok(CitationVerificationResultRecord {
        id: parse_uuid(&get_text(row, 0), "citation_verification_results.id")?,
        run_id: parse_uuid(&get_text(row, 1), "citation_verification_results.run_id")?,
        citation_text: get_text(row, 2),
        normalized_citation: get_text(row, 3),
        status: parse_citation_status(&status_raw)?,
        provider_reference: get_opt_text(row, 5),
        provider_title: get_opt_text(row, 6),
        detail: get_opt_text(row, 7),
        waived_by: get_opt_text(row, 8),
        waiver_reason: get_opt_text(row, 9),
        waived_at: get_opt_text(row, 10)
            .map(|value| parse_timestamp(&value))
            .transpose()
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        created_at: parse_timestamp(&get_text(row, 11))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_account(row: &libsql::Row) -> Result<TrustAccountRecord, DatabaseError> {
    Ok(TrustAccountRecord {
        id: parse_uuid(&get_text(row, 0), "trust_accounts.id")?,
        user_id: get_text(row, 1),
        name: get_text(row, 2),
        bank_name: get_opt_text(row, 3),
        account_number_last4: get_opt_text(row, 4),
        is_primary: get_i64(row, 5) != 0,
        created_at: parse_timestamp(&get_text(row, 6))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 7))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_statement_import(
    row: &libsql::Row,
) -> Result<TrustStatementImportRecord, DatabaseError> {
    Ok(TrustStatementImportRecord {
        id: parse_uuid(&get_text(row, 0), "trust_statement_imports.id")?,
        user_id: get_text(row, 1),
        trust_account_id: parse_uuid(
            &get_text(row, 2),
            "trust_statement_imports.trust_account_id",
        )?,
        statement_date: parse_naive_date(&get_text(row, 3), "statement_date")?,
        starting_balance: get_decimal(row, 4),
        ending_balance: get_decimal(row, 5),
        imported_by: get_text(row, 6),
        row_count: i32::try_from(get_i64(row, 7))
            .map_err(|_| DatabaseError::Serialization("invalid row_count".to_string()))?,
        created_at: parse_timestamp(&get_text(row, 8))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_statement_line(
    row: &libsql::Row,
) -> Result<TrustStatementLineRecord, DatabaseError> {
    Ok(TrustStatementLineRecord {
        id: parse_uuid(&get_text(row, 0), "trust_statement_lines.id")?,
        statement_import_id: parse_uuid(
            &get_text(row, 1),
            "trust_statement_lines.statement_import_id",
        )?,
        entry_date: parse_naive_date(&get_text(row, 2), "entry_date")?,
        description: get_text(row, 3),
        debit: get_decimal(row, 4),
        credit: get_decimal(row, 5),
        running_balance: get_decimal(row, 6),
        reference_number: get_opt_text(row, 7),
        created_at: parse_timestamp(&get_text(row, 8))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_reconciliation(
    row: &libsql::Row,
) -> Result<TrustReconciliationRecord, DatabaseError> {
    let status_raw = get_text(row, 8);
    let exceptions_json = serde_json::from_str(&get_text(row, 7))
        .map_err(|e| DatabaseError::Serialization(e.to_string()))?;
    Ok(TrustReconciliationRecord {
        id: parse_uuid(&get_text(row, 0), "trust_reconciliations.id")?,
        user_id: get_text(row, 1),
        trust_account_id: parse_uuid(&get_text(row, 2), "trust_reconciliations.trust_account_id")?,
        statement_import_id: parse_uuid(
            &get_text(row, 3),
            "trust_reconciliations.statement_import_id",
        )?,
        statement_ending_balance: get_decimal(row, 4),
        book_balance: get_decimal(row, 5),
        client_balance_total: get_decimal(row, 6),
        difference: get_decimal(row, 9),
        exceptions_json,
        status: parse_trust_reconciliation_status(&status_raw)?,
        signed_off_by: get_opt_text(row, 10),
        signed_off_at: get_opt_text(row, 11)
            .map(|value| parse_timestamp(&value))
            .transpose()
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        created_at: parse_timestamp(&get_text(row, 12))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
        updated_at: parse_timestamp(&get_text(row, 13))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

fn row_to_trust_ledger_entry(row: &libsql::Row) -> Result<TrustLedgerEntryRecord, DatabaseError> {
    let entry_type_raw = get_text(row, 3);
    let entry_detail = get_opt_text(row, 11);
    let entry_type =
        crate::db::TrustLedgerEntryType::from_db_columns(&entry_type_raw, entry_detail.as_deref())
            .ok_or_else(|| {
                DatabaseError::Serialization(format!(
                    "invalid trust ledger entry type '{}' / detail {:?}",
                    entry_type_raw, entry_detail
                ))
            })?;
    Ok(TrustLedgerEntryRecord {
        id: parse_uuid(&get_text(row, 0), "trust_ledger.id")?,
        user_id: get_text(row, 1),
        matter_id: get_text(row, 2),
        trust_account_id: get_opt_text(row, 10)
            .map(|value| parse_uuid(&value, "trust_ledger.trust_account_id"))
            .transpose()?,
        entry_type,
        amount: get_decimal(row, 4),
        delta: get_decimal(row, 12),
        balance_after: get_decimal(row, 5),
        description: get_text(row, 6),
        reference_number: get_opt_text(row, 13),
        source: get_opt_text(row, 14)
            .and_then(|value| crate::db::TrustLedgerSource::from_db_value(&value))
            .unwrap_or_default(),
        invoice_id: get_opt_text(row, 7)
            .map(|value| parse_uuid(&value, "trust_ledger.invoice_id"))
            .transpose()?,
        recorded_by: get_text(row, 8),
        created_at: parse_timestamp(&get_text(row, 9))
            .map_err(|e| DatabaseError::Serialization(e.to_string()))?,
    })
}

#[async_trait::async_trait]
impl BillingRateStore for LibSqlBackend {
    async fn list_billing_rate_schedules(
        &self,
        user_id: &str,
        matter_id: Option<&str>,
        timekeeper: Option<&str>,
    ) -> Result<Vec<BillingRateScheduleRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules \
                 WHERE user_id = ?1 \
                   AND (?2 IS NULL OR matter_id = ?2) \
                   AND (?3 IS NULL OR timekeeper = ?3) \
                 ORDER BY COALESCE(matter_id, ''), timekeeper, effective_start DESC",
                params![user_id, opt_text(matter_id), opt_text(timekeeper)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_billing_rate_schedule(&row)?);
        }
        Ok(out)
    }

    async fn create_billing_rate_schedule(
        &self,
        user_id: &str,
        input: &CreateBillingRateScheduleParams,
    ) -> Result<BillingRateScheduleRecord, DatabaseError> {
        let conn = self.connect().await?;
        let schedule_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO billing_rate_schedules \
             (id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'), datetime('now'))",
            params![
                schedule_id.as_str(),
                user_id,
                opt_text(input.matter_id.as_deref()),
                input.timekeeper.as_str(),
                input.rate.to_string(),
                input.effective_start.to_string(),
                opt_text_owned(input.effective_end.map(|value| value.to_string())),
            ],
        )
        .await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules WHERE id = ?1 LIMIT 1",
                params![schedule_id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load billing rate schedule".to_string()))?;
        row_to_billing_rate_schedule(&row)
    }

    async fn update_billing_rate_schedule(
        &self,
        user_id: &str,
        schedule_id: Uuid,
        input: &UpdateBillingRateScheduleParams,
    ) -> Result<Option<BillingRateScheduleRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let existing = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, schedule_id.to_string()],
            )
            .await?
            .next()
            .await?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        let existing = row_to_billing_rate_schedule(&existing)?;
        let matter_id = input
            .matter_id
            .clone()
            .unwrap_or(existing.matter_id.clone());
        let timekeeper = input
            .timekeeper
            .clone()
            .unwrap_or(existing.timekeeper.clone());
        let rate = input.rate.unwrap_or(existing.rate);
        let effective_start = input.effective_start.unwrap_or(existing.effective_start);
        let effective_end = input.effective_end.unwrap_or(existing.effective_end);
        conn.execute(
            "UPDATE billing_rate_schedules SET \
               matter_id = ?3, timekeeper = ?4, rate = ?5, effective_start = ?6, effective_end = ?7, updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                schedule_id.to_string(),
                opt_text(matter_id.as_deref()),
                timekeeper.as_str(),
                rate.to_string(),
                effective_start.to_string(),
                opt_text_owned(effective_end.map(|value| value.to_string())),
            ],
        )
        .await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, timekeeper, rate, effective_start, effective_end, created_at, updated_at \
                 FROM billing_rate_schedules WHERE id = ?1 LIMIT 1",
                params![schedule_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to reload billing rate schedule".to_string()))?;
        Ok(Some(row_to_billing_rate_schedule(&row)?))
    }
}

#[async_trait::async_trait]
impl CitationVerificationStore for LibSqlBackend {
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
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;

        let op_result: Result<
            (
                CitationVerificationRunRecord,
                Vec<CitationVerificationResultRecord>,
            ),
            DatabaseError,
        > = async {
            let run_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO citation_verification_runs \
                 (id, user_id, matter_id, matter_document_id, provider, document_hash, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
                params![
                    run_id.as_str(),
                    user_id,
                    input.matter_id.as_str(),
                    input.matter_document_id.to_string(),
                    input.provider.as_str(),
                    input.document_hash.as_str(),
                    input.created_by.as_str(),
                ],
            )
            .await?;
            let run_row = conn
                .query(
                    "SELECT id, user_id, matter_id, matter_document_id, provider, document_hash, created_by, created_at \
                     FROM citation_verification_runs WHERE id = ?1 LIMIT 1",
                    params![run_id.as_str()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| DatabaseError::Query("failed to load citation run".to_string()))?;
            let run = row_to_citation_run(&run_row)?;

            let mut stored_results = Vec::new();
            for result in results {
                let result_id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO citation_verification_results \
                     (id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), datetime('now'))",
                    params![
                        result_id.as_str(),
                        run_id.as_str(),
                        result.citation_text.as_str(),
                        result.normalized_citation.as_str(),
                        result.status.as_str(),
                        opt_text(result.provider_reference.as_deref()),
                        opt_text(result.provider_title.as_deref()),
                        opt_text(result.detail.as_deref()),
                        opt_text(result.waived_by.as_deref()),
                        opt_text(result.waiver_reason.as_deref()),
                        opt_text_owned(result.waived_at.as_ref().map(fmt_ts)),
                    ],
                )
                .await?;
                let row = conn
                    .query(
                        "SELECT id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at, created_at, updated_at \
                         FROM citation_verification_results WHERE id = ?1 LIMIT 1",
                        params![result_id.as_str()],
                    )
                    .await?
                    .next()
                    .await?
                    .ok_or_else(|| DatabaseError::Query("failed to load citation result".to_string()))?;
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
            conn.execute(
                "UPDATE matter_documents SET readiness_state = ?3, updated_at = datetime('now') \
                 WHERE user_id = ?1 AND id = ?2",
                params![
                    user_id,
                    input.matter_document_id.to_string(),
                    readiness_state.as_str(),
                ],
            )
            .await?;
            Ok((run, stored_results))
        }
        .await;

        match op_result {
            Ok(value) => {
                conn.execute("COMMIT", ()).await?;
                Ok(value)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn latest_citation_verification_run(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Option<CitationVerificationRunRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, matter_id, matter_document_id, provider, document_hash, created_by, created_at \
                 FROM citation_verification_runs \
                 WHERE user_id = ?1 AND matter_document_id = ?2 \
                 ORDER BY created_at DESC, rowid DESC \
                 LIMIT 1",
                params![user_id, matter_document_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_citation_run(&row)).transpose()
    }

    async fn list_citation_verification_results(
        &self,
        user_id: &str,
        matter_document_id: Uuid,
    ) -> Result<Vec<CitationVerificationResultRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, run_id, citation_text, normalized_citation, status, provider_reference, provider_title, detail, waived_by, waiver_reason, waived_at, created_at, updated_at \
                 FROM citation_verification_results \
                 WHERE run_id = ( \
                     SELECT id FROM citation_verification_runs \
                     WHERE user_id = ?1 AND matter_document_id = ?2 \
                     ORDER BY created_at DESC, rowid DESC LIMIT 1 \
                 ) \
                 ORDER BY created_at ASC, rowid ASC",
                params![user_id, matter_document_id.to_string()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_citation_result(&row)?);
        }
        Ok(out)
    }

    async fn set_matter_document_readiness(
        &self,
        user_id: &str,
        matter_id: &str,
        matter_document_id: Uuid,
        state: DocumentReadinessState,
    ) -> Result<Option<MatterDocumentRecord>, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE matter_documents SET readiness_state = ?4, updated_at = datetime('now') \
             WHERE user_id = ?1 AND matter_id = ?2 AND id = ?3",
            params![
                user_id,
                matter_id,
                matter_document_id.to_string(),
                state.as_str()
            ],
        )
        .await?;
        self.get_matter_document(user_id, matter_id, matter_document_id)
            .await
    }
}

#[async_trait::async_trait]
impl TrustAccountingStore for LibSqlBackend {
    async fn get_primary_trust_account(
        &self,
        user_id: &str,
    ) -> Result<Option<TrustAccountRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at \
                 FROM trust_accounts WHERE user_id = ?1 AND is_primary = 1 LIMIT 1",
                params![user_id],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_trust_account(&row)).transpose()
    }

    async fn upsert_primary_trust_account(
        &self,
        user_id: &str,
        input: &UpsertTrustAccountParams,
    ) -> Result<TrustAccountRecord, DatabaseError> {
        let conn = self.connect().await?;
        if let Some(existing) = self.get_primary_trust_account(user_id).await? {
            conn.execute(
                "UPDATE trust_accounts SET name = ?3, bank_name = ?4, account_number_last4 = ?5, updated_at = datetime('now') \
                 WHERE user_id = ?1 AND id = ?2",
                params![
                    user_id,
                    existing.id.to_string(),
                    input.name.as_str(),
                    opt_text(input.bank_name.as_deref()),
                    opt_text(input.account_number_last4.as_deref()),
                ],
            )
            .await?;
            let row = conn
                .query(
                    "SELECT id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at \
                     FROM trust_accounts WHERE id = ?1 LIMIT 1",
                    params![existing.id.to_string()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| DatabaseError::Query("failed to reload trust account".to_string()))?;
            return row_to_trust_account(&row);
        }

        let account_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO trust_accounts \
             (id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, 1, datetime('now'), datetime('now'))",
            params![
                account_id.as_str(),
                user_id,
                input.name.as_str(),
                opt_text(input.bank_name.as_deref()),
                opt_text(input.account_number_last4.as_deref()),
            ],
        )
        .await?;
        let row = conn
            .query(
                "SELECT id, user_id, name, bank_name, account_number_last4, is_primary, created_at, updated_at \
                 FROM trust_accounts WHERE id = ?1 LIMIT 1",
                params![account_id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load trust account".to_string()))?;
        row_to_trust_account(&row)
    }

    async fn list_trust_ledger_entries_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Vec<TrustLedgerEntryRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_id, matter_id, entry_type, amount, balance_after, description, invoice_id, recorded_by, created_at, trust_account_id, entry_detail, delta, reference_number, source \
                 FROM trust_ledger \
                 WHERE user_id = ?1 AND trust_account_id = ?2 \
                 ORDER BY created_at DESC, rowid DESC",
                params![user_id, trust_account_id.to_string()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_trust_ledger_entry(&row)?);
        }
        Ok(out)
    }

    async fn current_trust_account_balance(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Decimal, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT COALESCE((SELECT balance_after FROM trust_ledger WHERE user_id = ?1 AND trust_account_id = ?2 ORDER BY created_at DESC, rowid DESC LIMIT 1), '0') AS balance",
                params![user_id, trust_account_id.to_string()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to read trust account balance".to_string()))?;
        get_text(&row, 0)
            .parse::<Decimal>()
            .map_err(|_| DatabaseError::Serialization("invalid trust account balance".to_string()))
    }

    async fn import_trust_statement(
        &self,
        user_id: &str,
        input: &CreateTrustStatementImportParams,
        lines: &[CreateTrustStatementLineParams],
    ) -> Result<(TrustStatementImportRecord, Vec<TrustStatementLineRecord>), DatabaseError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN", ()).await?;
        let op_result: Result<
            (
                TrustStatementImportRecord,
                Vec<TrustStatementLineRecord>,
            ),
            DatabaseError,
        > = async {
            let import_id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO trust_statement_imports \
                 (id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                params![
                    import_id.as_str(),
                    user_id,
                    input.trust_account_id.to_string(),
                    input.statement_date.to_string(),
                    input.starting_balance.to_string(),
                    input.ending_balance.to_string(),
                    input.imported_by.as_str(),
                    i64::try_from(lines.len()).unwrap_or(0),
                ],
            )
            .await?;
            let import_row = conn
                .query(
                    "SELECT id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count, created_at \
                     FROM trust_statement_imports WHERE id = ?1 LIMIT 1",
                    params![import_id.as_str()],
                )
                .await?
                .next()
                .await?
                .ok_or_else(|| DatabaseError::Query("failed to load trust statement import".to_string()))?;
            let import = row_to_trust_statement_import(&import_row)?;
            let mut stored_lines = Vec::new();
            for line in lines {
                let line_id = Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO trust_statement_lines \
                     (id, statement_import_id, entry_date, description, debit, credit, running_balance, reference_number, created_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, datetime('now'))",
                    params![
                        line_id.as_str(),
                        import_id.as_str(),
                        line.entry_date.to_string(),
                        line.description.as_str(),
                        line.debit.to_string(),
                        line.credit.to_string(),
                        line.running_balance.to_string(),
                        opt_text(line.reference_number.as_deref()),
                    ],
                )
                .await?;
                let row = conn
                    .query(
                        "SELECT id, statement_import_id, entry_date, description, debit, credit, running_balance, reference_number, created_at \
                         FROM trust_statement_lines WHERE id = ?1 LIMIT 1",
                        params![line_id.as_str()],
                    )
                    .await?
                    .next()
                    .await?
                    .ok_or_else(|| DatabaseError::Query("failed to load trust statement line".to_string()))?;
                stored_lines.push(row_to_trust_statement_line(&row)?);
            }
            Ok((import, stored_lines))
        }
        .await;

        match op_result {
            Ok(value) => {
                conn.execute("COMMIT", ()).await?;
                Ok(value)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    async fn get_trust_statement_import(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Option<TrustStatementImportRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, trust_account_id, statement_date, starting_balance, ending_balance, imported_by, row_count, created_at \
                 FROM trust_statement_imports WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, statement_import_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_trust_statement_import(&row))
            .transpose()
    }

    async fn list_trust_statement_lines(
        &self,
        user_id: &str,
        statement_import_id: Uuid,
    ) -> Result<Vec<TrustStatementLineRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT tsl.id, tsl.statement_import_id, tsl.entry_date, tsl.description, tsl.debit, tsl.credit, tsl.running_balance, tsl.reference_number, tsl.created_at \
                 FROM trust_statement_lines tsl \
                 JOIN trust_statement_imports tsi ON tsi.id = tsl.statement_import_id \
                 WHERE tsi.user_id = ?1 AND tsl.statement_import_id = ?2 \
                 ORDER BY tsl.entry_date ASC, tsl.created_at ASC",
                params![user_id, statement_import_id.to_string()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(row_to_trust_statement_line(&row)?);
        }
        Ok(out)
    }

    async fn compute_trust_reconciliation(
        &self,
        user_id: &str,
        input: &ComputeTrustReconciliationParams,
    ) -> Result<TrustReconciliationRecord, DatabaseError> {
        let conn = self.connect().await?;
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

        let mut client_balance_total = Decimal::ZERO;
        let mut matter_balances: std::collections::BTreeMap<String, Decimal> =
            std::collections::BTreeMap::new();
        for entry in &ledger_entries {
            let balance = matter_balances
                .entry(entry.matter_id.clone())
                .or_insert(Decimal::ZERO);
            *balance += entry.delta;
        }
        for balance in matter_balances.values() {
            client_balance_total += *balance;
        }

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
        let reconciliation_id = Uuid::new_v4().to_string();
        let exceptions_json = serde_json::Value::Array(exceptions);
        conn.execute(
            "INSERT INTO trust_reconciliations \
             (id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'draft', ?9, NULL, NULL, datetime('now'), datetime('now'))",
            params![
                reconciliation_id.as_str(),
                user_id,
                input.trust_account_id.to_string(),
                input.statement_import_id.to_string(),
                statement_import.ending_balance.to_string(),
                book_balance.to_string(),
                client_balance_total.to_string(),
                exceptions_json.to_string(),
                difference.to_string(),
            ],
        )
        .await?;
        let row = conn
            .query(
                "SELECT id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at \
                 FROM trust_reconciliations WHERE id = ?1 LIMIT 1",
                params![reconciliation_id.as_str()],
            )
            .await?
            .next()
            .await?
            .ok_or_else(|| DatabaseError::Query("failed to load trust reconciliation".to_string()))?;
        row_to_trust_reconciliation(&row)
    }

    async fn get_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at \
                 FROM trust_reconciliations WHERE user_id = ?1 AND id = ?2 LIMIT 1",
                params![user_id, reconciliation_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_trust_reconciliation(&row)).transpose()
    }

    async fn latest_trust_reconciliation_for_account(
        &self,
        user_id: &str,
        trust_account_id: Uuid,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.connect().await?;
        let row = conn
            .query(
                "SELECT id, user_id, trust_account_id, statement_import_id, statement_ending_balance, book_balance, client_balance_total, exceptions_json, status, difference, signed_off_by, signed_off_at, created_at, updated_at \
                 FROM trust_reconciliations \
                 WHERE user_id = ?1 AND trust_account_id = ?2 \
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![user_id, trust_account_id.to_string()],
            )
            .await?
            .next()
            .await?;
        row.map(|row| row_to_trust_reconciliation(&row)).transpose()
    }

    async fn signoff_trust_reconciliation(
        &self,
        user_id: &str,
        reconciliation_id: Uuid,
        signed_off_by: &str,
    ) -> Result<Option<TrustReconciliationRecord>, DatabaseError> {
        let conn = self.connect().await?;
        conn.execute(
            "UPDATE trust_reconciliations SET status = 'signed_off', signed_off_by = ?3, signed_off_at = ?4, updated_at = datetime('now') \
             WHERE user_id = ?1 AND id = ?2",
            params![
                user_id,
                reconciliation_id.to_string(),
                signed_off_by,
                Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ],
        )
        .await?;
        self.get_trust_reconciliation(user_id, reconciliation_id)
            .await
    }
}
