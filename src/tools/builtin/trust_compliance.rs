//! Ontario trust-compliance advisory tool.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

use crate::context::JobContext;
use crate::db::{Database, TrustLedgerEntryType};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct TrustComplianceCheckerTool {
    store: Option<Arc<dyn Database>>,
}

impl TrustComplianceCheckerTool {
    pub fn new(store: Option<Arc<dyn Database>>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TrustComplianceCheckerTool {
    fn name(&self) -> &str {
        "trust_compliance_checker"
    }

    fn description(&self) -> &str {
        "Run advisory trust-account compliance checks against stored trust ledgers and reconciliations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "account_id": {
                    "type": "string",
                    "description": "Trust account UUID"
                },
                "check_type": {
                    "type": "string",
                    "enum": ["commingling", "prompt_deposit", "disbursement_limit", "reconciliation"],
                    "description": "Which trust compliance check to run"
                }
            },
            "required": ["account_id", "check_type"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let Some(store) = self.store.as_ref() else {
            return Ok(ToolOutput::text(
                "Trust compliance checking is running without a database store; only live advisory mode is available in this context.",
                start.elapsed(),
            ));
        };

        let account_id = Uuid::parse_str(require_str(&params, "account_id")?).map_err(|err| {
            ToolError::InvalidParameters(format!("account_id must be a UUID: {err}"))
        })?;
        let check_type = require_str(&params, "check_type")?;
        let entries = store
            .list_trust_ledger_entries_for_account(&ctx.user_id, account_id)
            .await
            .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;

        let (compliant, violations, recommendation) = match check_type {
            "commingling" => evaluate_commingling(&entries),
            "prompt_deposit" => evaluate_prompt_deposit(&entries),
            "disbursement_limit" => evaluate_disbursement_limits(&entries),
            "reconciliation" => {
                let latest = store
                    .latest_trust_reconciliation_for_account(&ctx.user_id, account_id)
                    .await
                    .map_err(|err| ToolError::ExecutionFailed(err.to_string()))?;
                evaluate_reconciliation(latest.as_ref())
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported check_type '{other}'"
                )));
            }
        };

        Ok(ToolOutput::success(
            serde_json::json!({
                "account_id": account_id.to_string(),
                "check_type": check_type,
                "compliant": compliant,
                "violations": violations,
                "recommendation": recommendation,
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

fn evaluate_commingling(
    entries: &[crate::db::TrustLedgerEntryRecord],
) -> (bool, Vec<String>, String) {
    let violations = entries
        .iter()
        .filter(|entry| entry.entry_type == TrustLedgerEntryType::FirmFunds)
        .filter(|entry| {
            let description = entry.description.to_ascii_lowercase();
            !description.contains("fee") && !description.contains("buffer")
        })
        .map(|entry| {
            format!(
                "Firm funds entry '{}' on {} should be reviewed for commingling risk.",
                entry.description,
                entry.created_at.date_naive()
            )
        })
        .collect::<Vec<_>>();
    (
        violations.is_empty(),
        violations,
        "Keep firm-funds entries limited to documented bank-fee buffers and record the rationale in the ledger.".to_string(),
    )
}

fn evaluate_prompt_deposit(
    entries: &[crate::db::TrustLedgerEntryRecord],
) -> (bool, Vec<String>, String) {
    let deposits = entries
        .iter()
        .filter(|entry| entry.entry_type == TrustLedgerEntryType::Deposit)
        .count();
    let violations = if deposits == 0 {
        vec!["No trust deposits were available for prompt-deposit review.".to_string()]
    } else {
        Vec::new()
    };
    (
        violations.is_empty(),
        violations,
        "This check is advisory: the current ledger stores created_at but not separate received_date/deposit_date fields, so prompt-deposit timing still needs manual review.".to_string(),
    )
}

fn evaluate_disbursement_limits(
    entries: &[crate::db::TrustLedgerEntryRecord],
) -> (bool, Vec<String>, String) {
    let mut by_matter = BTreeMap::<String, rust_decimal::Decimal>::new();
    let mut violations = Vec::new();

    let mut ordered = entries.to_vec();
    ordered.sort_by_key(|entry| entry.created_at);
    for entry in ordered {
        let balance = by_matter
            .entry(entry.matter_id.clone())
            .or_insert(rust_decimal::Decimal::ZERO);
        *balance += entry.delta;
        if *balance < rust_decimal::Decimal::ZERO {
            violations.push(format!(
                "Matter '{}' dropped below zero after '{}' on {}.",
                entry.matter_id,
                entry.description,
                entry.created_at.date_naive()
            ));
        }
    }

    (
        violations.is_empty(),
        violations,
        "Trust disbursements should never overdraw a client matter ledger.".to_string(),
    )
}

fn evaluate_reconciliation(
    latest: Option<&crate::db::TrustReconciliationRecord>,
) -> (bool, Vec<String>, String) {
    let mut violations = Vec::new();
    match latest {
        None => violations
            .push("No trust reconciliation has been recorded for this account.".to_string()),
        Some(record) => {
            let signed_at = record.signed_off_at.unwrap_or(record.updated_at);
            if (Utc::now() - signed_at).num_days() > 35 {
                violations.push(format!(
                    "Latest reconciliation is older than 35 days (last activity {}).",
                    signed_at.date_naive()
                ));
            }
            if !matches!(
                record.status,
                crate::db::TrustReconciliationStatus::SignedOff
            ) {
                violations.push("Latest reconciliation is not signed off.".to_string());
            }
        }
    }

    (
        violations.is_empty(),
        violations,
        "Law Society reconciliation review should be current and signed off at least monthly."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use super::*;
    use crate::db::{ClientType, MatterStatus, UpsertMatterParams, UserRole};

    #[test]
    fn schema_contains_required_fields() {
        let tool = TrustComplianceCheckerTool::new(None);
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.contains(&serde_json::Value::String("account_id".to_string())));
        assert!(required.contains(&serde_json::Value::String("check_type".to_string())));
    }

    #[tokio::test]
    async fn no_store_returns_advisory_message() {
        let tool = TrustComplianceCheckerTool::new(None);
        let output = tool
            .execute(
                serde_json::json!({
                    "account_id": Uuid::new_v4().to_string(),
                    "check_type": "reconciliation"
                }),
                &JobContext::default(),
            )
            .await
            .expect("advisory message should succeed");

        assert!(
            output
                .result
                .as_str()
                .expect("text result")
                .contains("without a database store")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn commingling_check_flags_firm_funds_entries() {
        let (db, _dir) = crate::testing::test_db().await;
        db.ensure_user_account("default", "Default User", UserRole::Admin)
            .await
            .expect("user");
        let client = db
            .upsert_client_by_normalized_name(
                "default",
                &crate::db::CreateClientParams {
                    name: "Test Client".to_string(),
                    client_type: ClientType::Entity,
                    email: None,
                    phone: None,
                    address: None,
                    notes: None,
                },
            )
            .await
            .expect("client");
        db.upsert_matter(
            "default",
            &UpsertMatterParams {
                matter_id: "matter-1".to_string(),
                client_id: client.id,
                status: MatterStatus::Active,
                stage: None,
                practice_area: Some("litigation".to_string()),
                jurisdiction: Some("ON".to_string()),
                opened_at: None,
                closed_at: None,
                assigned_to: vec![],
                custom_fields: serde_json::json!({}),
            },
        )
        .await
        .expect("matter");
        let account = db
            .upsert_primary_trust_account(
                "default",
                &crate::db::UpsertTrustAccountParams {
                    name: "Primary IOLTA".to_string(),
                    bank_name: Some("Bank".to_string()),
                    account_number_last4: Some("1234".to_string()),
                },
            )
            .await
            .expect("account");
        db.append_trust_ledger_entry(
            "default",
            "matter-1",
            &crate::db::CreateTrustLedgerEntryParams {
                trust_account_id: Some(account.id),
                entry_type: TrustLedgerEntryType::FirmFunds,
                amount: dec!(25.00),
                delta: dec!(25.00),
                description: "Operating transfer".to_string(),
                reference_number: None,
                source: crate::db::TrustLedgerSource::Manual,
                invoice_id: None,
                recorded_by: "tester".to_string(),
            },
        )
        .await
        .expect("entry");

        let tool = TrustComplianceCheckerTool::new(Some(db));
        let output = tool
            .execute(
                serde_json::json!({
                    "account_id": account.id.to_string(),
                    "check_type": "commingling"
                }),
                &JobContext::default(),
            )
            .await
            .expect("check should succeed");

        assert_eq!(output.result["compliant"], false);
        assert_eq!(
            output.result["violations"]
                .as_array()
                .expect("violations")
                .len(),
            1
        );
    }
}
