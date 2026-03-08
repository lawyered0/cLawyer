//! Ontario court forms lookup tool.

use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::Instant;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct OntarioCourtFormTool;

#[derive(Debug, Clone)]
struct FormMetadata {
    title: &'static str,
    rule_reference: &'static str,
    required_fields: &'static [&'static str],
    instructions: &'static str,
    source_url: &'static str,
}

static ONTARIO_FORMS: LazyLock<HashMap<&'static str, FormMetadata>> = LazyLock::new(|| {
    HashMap::from([
        (
            "4A",
            FormMetadata {
                title: "Notice of Change of Representation",
                rule_reference: "r 15.04",
                required_fields: &[
                    "court file number",
                    "party name",
                    "new representative",
                    "service details",
                ],
                instructions: "Use when a party changes lawyers or moves from representation to self-representation.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "14A",
            FormMetadata {
                title: "Statement of Claim (General)",
                rule_reference: "r 14.01",
                required_fields: &[
                    "court file number",
                    "parties",
                    "claim details",
                    "relief claimed",
                ],
                instructions: "Set out the material facts, the relief claimed, and the basis for jurisdiction and costs.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "14B",
            FormMetadata {
                title: "Statement of Claim (Mortgage)",
                rule_reference: "r 14.01",
                required_fields: &[
                    "court file number",
                    "mortgage details",
                    "default details",
                    "relief claimed",
                ],
                instructions: "Use for mortgage enforcement claims requiring the mortgage-specific pleading form.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "16A",
            FormMetadata {
                title: "Statement of Defence",
                rule_reference: "r 18.01",
                required_fields: &[
                    "court file number",
                    "parties",
                    "admissions and denials",
                    "affirmative defences",
                ],
                instructions: "Respond paragraph by paragraph, admit what is admitted, deny what is denied, and plead any positive defences.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "29A",
            FormMetadata {
                title: "Notice of Examination",
                rule_reference: "r 34.04",
                required_fields: &[
                    "court file number",
                    "person to be examined",
                    "date",
                    "time",
                    "location",
                ],
                instructions: "Use to schedule an examination for discovery or examination of a witness under the rule.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "34A",
            FormMetadata {
                title: "Affidavit",
                rule_reference: "r 4.06(1)",
                required_fields: &["deponent", "facts sworn", "commissioning details"],
                instructions: "Set out facts in numbered paragraphs and ensure the affidavit is properly commissioned.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
        (
            "59A",
            FormMetadata {
                title: "Order",
                rule_reference: "r 59.04",
                required_fields: &[
                    "court file number",
                    "style of cause",
                    "operative terms",
                    "judge or registrar signature line",
                ],
                instructions: "Prepare the operative terms clearly and match the endorsed disposition or draft approval process.",
                source_url: "https://www.ontario.ca/laws/regulation/120194",
            },
        ),
    ])
});

#[async_trait]
impl Tool for OntarioCourtFormTool {
    fn name(&self) -> &str {
        "ontario_court_form"
    }

    fn description(&self) -> &str {
        "Look up Ontario court form metadata and filing guidance by form number."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "form_number": {
                    "type": "string",
                    "description": "Ontario court form number, e.g. '14A' or '16A'"
                }
            },
            "required": ["form_number"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let key = require_str(&params, "form_number")?
            .trim()
            .to_ascii_uppercase();

        if let Some(form) = ONTARIO_FORMS.get(key.as_str()) {
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "form_number": key,
                    "title": form.title,
                    "rule_reference": form.rule_reference,
                    "required_fields": form.required_fields,
                    "instructions": form.instructions,
                    "source_url": form.source_url,
                }),
                start.elapsed(),
            ));
        }

        Ok(ToolOutput::text(
            format!(
                "Form {} is not in the local registry. Check the official Ontario forms regulation at https://www.ontario.ca/laws/regulation/120194.",
                key
            ),
            start.elapsed(),
        ))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn form_14a_found() {
        let tool = OntarioCourtFormTool;
        let output = tool
            .execute(
                serde_json::json!({ "form_number": "14A" }),
                &JobContext::default(),
            )
            .await
            .expect("form lookup should succeed");

        assert!(
            output.result["title"]
                .as_str()
                .expect("title")
                .contains("Statement of Claim")
        );
    }

    #[tokio::test]
    async fn form_unknown_graceful() {
        let tool = OntarioCourtFormTool;
        let output = tool
            .execute(
                serde_json::json!({ "form_number": "99Z" }),
                &JobContext::default(),
            )
            .await
            .expect("unknown form should still succeed");

        assert!(
            output
                .result
                .as_str()
                .expect("text result")
                .contains("not in the local registry")
        );
    }
}
