//! Court rule listing and deadline calculation tools.

use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};

use crate::context::JobContext;
use crate::legal::calendar::{
    DeadlineProvider, FirstPartyProvider, apply_rule_with_trace, get_court_rule,
};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct CourtDeadlineCalculatorTool;

pub struct ListCourtRulesTool;

#[async_trait]
impl Tool for CourtDeadlineCalculatorTool {
    fn name(&self) -> &str {
        "court_deadline_calculator"
    }

    fn description(&self) -> &str {
        "Calculate a court deadline from a bundled court rule ID and trigger date."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "rule_id": {
                    "type": "string",
                    "description": "Court rule ID, e.g. 'frcp_12_a_1' or 'on_rcp_48_01'"
                },
                "trigger_date": {
                    "type": "string",
                    "description": "Triggering event date in YYYY-MM-DD format"
                }
            },
            "required": ["rule_id", "trigger_date"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let rule_id = require_str(&params, "rule_id")?;
        let trigger_date = parse_date(require_str(&params, "trigger_date")?)?;

        let rule = get_court_rule(rule_id)
            .map_err(ToolError::ExecutionFailed)?
            .ok_or_else(|| ToolError::InvalidParameters(invalid_rule_message(rule_id)))?;
        let trigger_dt = midnight_utc(trigger_date)?;
        let (due_at, trace) = apply_rule_with_trace(&rule, trigger_dt);

        Ok(ToolOutput::success(
            serde_json::json!({
                "due_date": due_at.date_naive().to_string(),
                "rule_id": rule.id,
                "citation": rule.citation,
                "jurisdiction": rule.jurisdiction,
                "explanation": trace
            }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[async_trait]
impl Tool for ListCourtRulesTool {
    fn name(&self) -> &str {
        "list_court_rules"
    }

    fn description(&self) -> &str {
        "List all bundled court deadline rules with citation and jurisdiction metadata."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let rules =
            crate::legal::calendar::all_court_rules().map_err(ToolError::ExecutionFailed)?;
        let payload = rules
            .iter()
            .map(|rule| {
                serde_json::json!({
                    "id": rule.id,
                    "citation": rule.citation,
                    "jurisdiction": rule.jurisdiction,
                    "deadline_type": rule.deadline_type.as_str(),
                    "offset_days": rule.offset_days,
                    "court_days": rule.court_days,
                    "version": rule.version,
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolOutput::success(
            serde_json::json!({ "rules": payload }),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

fn parse_date(raw: &str) -> Result<NaiveDate, ToolError> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|err| ToolError::InvalidParameters(format!("invalid date '{raw}': {err}")))
}

fn midnight_utc(date: NaiveDate) -> Result<DateTime<Utc>, ToolError> {
    date.and_hms_opt(0, 0, 0)
        .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
        .ok_or_else(|| ToolError::ExecutionFailed("failed to construct UTC datetime".to_string()))
}

fn invalid_rule_message(rule_id: &str) -> String {
    let supported = FirstPartyProvider.supported_rule_ids().join(", ");
    format!("unknown rule_id '{rule_id}'. Valid rule IDs: {supported}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frcp_rule_lookup_found() {
        let tool = CourtDeadlineCalculatorTool;
        let output = tool
            .execute(
                serde_json::json!({
                    "rule_id": "frcp_12_a_1",
                    "trigger_date": "2026-03-02"
                }),
                &JobContext::default(),
            )
            .await
            .expect("deadline should compute");

        assert_eq!(output.result["rule_id"], "frcp_12_a_1");
        assert_eq!(output.result["due_date"], "2026-03-23");
    }

    #[tokio::test]
    async fn rule_lookup_not_found() {
        let tool = CourtDeadlineCalculatorTool;
        let err = tool
            .execute(
                serde_json::json!({
                    "rule_id": "missing_rule",
                    "trigger_date": "2026-03-02"
                }),
                &JobContext::default(),
            )
            .await
            .expect_err("missing rule should fail");

        assert!(matches!(err, ToolError::InvalidParameters(_)));
    }
}
