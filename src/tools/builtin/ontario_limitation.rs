//! Ontario limitation period calculator.

use std::time::Instant;

use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, Utc};

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct OntarioLimitationCalculatorTool;

#[async_trait]
impl Tool for OntarioLimitationCalculatorTool {
    fn name(&self) -> &str {
        "ontario_limitation_calculator"
    }

    fn description(&self) -> &str {
        "Calculate Ontario basic and ultimate limitation periods under the Limitations Act, 2002."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "discovery_date": {
                    "type": "string",
                    "description": "Discovery date in YYYY-MM-DD format"
                },
                "act_or_omission_date": {
                    "type": "string",
                    "description": "Act or omission date in YYYY-MM-DD format"
                },
                "involves_minor_or_disability": {
                    "type": "boolean",
                    "description": "Whether minority or disability tolling may apply"
                }
            },
            "required": ["discovery_date", "act_or_omission_date"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let discovery_date = parse_date(require_str(&params, "discovery_date")?)?;
        let act_date = parse_date(require_str(&params, "act_or_omission_date")?)?;
        let involves_disability = params
            .get("involves_minor_or_disability")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);

        let basic_expiry = add_years(discovery_date, 2)?;
        let ultimate_expiry = add_years(act_date, 15)?;
        let controlling_expiry = std::cmp::min(basic_expiry, ultimate_expiry);
        let today = Utc::now().date_naive();
        let limitation_expired = today > controlling_expiry;
        let days_remaining = if limitation_expired {
            0
        } else {
            (controlling_expiry - today).num_days()
        };
        let warning = involves_disability.then_some(
            "Minority or disability tolling may apply; manual legal review is required."
                .to_string(),
        );

        Ok(ToolOutput::success(
            serde_json::json!({
                "basic_limitation_expiry": basic_expiry.to_string(),
                "ultimate_limitation_expiry": ultimate_expiry.to_string(),
                "limitation_expired": limitation_expired,
                "days_remaining": days_remaining,
                "warning": warning
            }),
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

fn add_years(date: NaiveDate, years: i32) -> Result<NaiveDate, ToolError> {
    let target_year = date.year() + years;
    NaiveDate::from_ymd_opt(target_year, date.month(), date.day())
        .or_else(|| {
            let mut last_day = 31;
            while last_day > 27 {
                if let Some(candidate) =
                    NaiveDate::from_ymd_opt(target_year, date.month(), last_day)
                {
                    return Some(candidate);
                }
                last_day -= 1;
            }
            None
        })
        .ok_or_else(|| ToolError::ExecutionFailed("failed to add years to date".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn basic_two_year_limit() {
        let tool = OntarioLimitationCalculatorTool;
        let output = tool
            .execute(
                serde_json::json!({
                    "discovery_date": "2022-01-15",
                    "act_or_omission_date": "2021-06-01"
                }),
                &JobContext::default(),
            )
            .await
            .expect("limitation should compute");

        assert_eq!(output.result["basic_limitation_expiry"], "2024-01-15");
    }

    #[tokio::test]
    async fn ultimate_fifteen_year_limit() {
        let tool = OntarioLimitationCalculatorTool;
        let output = tool
            .execute(
                serde_json::json!({
                    "discovery_date": "2006-01-01",
                    "act_or_omission_date": "2005-03-01"
                }),
                &JobContext::default(),
            )
            .await
            .expect("limitation should compute");

        assert_eq!(output.result["ultimate_limitation_expiry"], "2020-03-01");
    }
}
