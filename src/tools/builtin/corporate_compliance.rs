//! Canadian corporate compliance checker for OBCA and CBCA.

use std::time::Instant;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct CorporateComplianceCheckerTool;

struct ComplianceTopic {
    keywords: &'static [&'static str],
    summary: &'static str,
    section_reference: &'static str,
}

const OBCA_TOPICS: &[ComplianceTopic] = &[
    ComplianceTopic {
        keywords: &["annual meeting", "agm", "meeting"],
        summary: "Annual meetings must be held within 15 months of the prior annual meeting and within six months of fiscal year-end.",
        section_reference: "OBCA, s 94",
    },
    ComplianceTopic {
        keywords: &["director election", "director elected", "election"],
        summary: "Directors are generally elected at each annual meeting unless a valid staggered-term structure has been adopted.",
        section_reference: "OBCA, ss 94, 119",
    },
    ComplianceTopic {
        keywords: &["director resignation", "resignation"],
        summary: "A director resignation must be in writing and is effective on delivery or the stated effective date.",
        section_reference: "OBCA, s 121(2)",
    },
    ComplianceTopic {
        keywords: &[
            "unanimous shareholder agreement",
            "usa",
            "shareholder agreement",
        ],
        summary: "A unanimous shareholder agreement can restrict or transfer directors' powers.",
        section_reference: "OBCA, s 108",
    },
    ComplianceTopic {
        keywords: &["oppression remedy", "oppression"],
        summary: "The oppression remedy is available to shareholders, creditors, directors, officers, and other proper complainants.",
        section_reference: "OBCA, s 248",
    },
];

const CBCA_TOPICS: &[ComplianceTopic] = &[
    ComplianceTopic {
        keywords: &["annual meeting", "agm", "meeting"],
        summary: "Annual meetings must be held within 15 months of the prior annual meeting and within six months of fiscal year-end.",
        section_reference: "CBCA, s 133",
    },
    ComplianceTopic {
        keywords: &["director residency", "resident canadian", "residency"],
        summary: "At least 25% of directors must generally be resident Canadians, subject to the small-board exception.",
        section_reference: "CBCA, s 105",
    },
    ComplianceTopic {
        keywords: &["articles of amendment", "share structure", "amendment"],
        summary: "Articles of amendment are required for share structure changes and other fundamental corporate amendments.",
        section_reference: "CBCA, s 173",
    },
    ComplianceTopic {
        keywords: &["continuance", "move jurisdiction", "continue into"],
        summary: "Continuance governs moving a corporation into or out of the CBCA framework.",
        section_reference: "CBCA, s 187",
    },
    ComplianceTopic {
        keywords: &["oppression remedy", "oppression"],
        summary: "The CBCA oppression remedy protects complainants from oppressive, unfairly prejudicial, or unfairly disregarding conduct.",
        section_reference: "CBCA, s 241",
    },
];

#[async_trait]
impl Tool for CorporateComplianceCheckerTool {
    fn name(&self) -> &str {
        "corporate_compliance_checker"
    }

    fn description(&self) -> &str {
        "Provide OBCA or CBCA corporate compliance guidance for common governance topics."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "act": {
                    "type": "string",
                    "enum": ["OBCA", "CBCA"],
                    "description": "Which corporate statute applies"
                },
                "topic": {
                    "type": "string",
                    "description": "Compliance topic, e.g. 'annual meeting' or 'director residency'"
                }
            },
            "required": ["act", "topic"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let act = require_str(&params, "act")?.trim().to_ascii_uppercase();
        let topic = require_str(&params, "topic")?.trim().to_ascii_lowercase();
        let topics = match act.as_str() {
            "OBCA" => OBCA_TOPICS,
            "CBCA" => CBCA_TOPICS,
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "unsupported act '{other}', expected OBCA or CBCA"
                )));
            }
        };

        if let Some(record) = topics
            .iter()
            .find(|entry| entry.keywords.iter().any(|keyword| topic.contains(keyword)))
        {
            return Ok(ToolOutput::success(
                serde_json::json!({
                    "act": act,
                    "topic_matched": record.keywords[0],
                    "summary": record.summary,
                    "section_reference": record.section_reference,
                    "caveat": "This is general information only. Consult legal counsel for advice specific to your corporation."
                }),
                start.elapsed(),
            ));
        }

        Ok(ToolOutput::text(
            format!(
                "No local compliance topic matched '{}' under {}. Try a narrower topic such as 'annual meeting', 'director residency', or 'oppression remedy'.",
                topic, act
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
    async fn obca_annual_meeting() {
        let tool = CorporateComplianceCheckerTool;
        let output = tool
            .execute(
                serde_json::json!({ "act": "OBCA", "topic": "annual meeting" }),
                &JobContext::default(),
            )
            .await
            .expect("compliance check should succeed");

        assert!(
            output.result["summary"]
                .as_str()
                .expect("summary")
                .contains("15 months")
        );
    }

    #[tokio::test]
    async fn cbca_director_residency() {
        let tool = CorporateComplianceCheckerTool;
        let output = tool
            .execute(
                serde_json::json!({ "act": "CBCA", "topic": "director residency" }),
                &JobContext::default(),
            )
            .await
            .expect("compliance check should succeed");

        assert!(
            output.result["summary"]
                .as_str()
                .expect("summary")
                .contains("25%")
        );
    }

    #[tokio::test]
    async fn unknown_topic_graceful() {
        let tool = CorporateComplianceCheckerTool;
        let output = tool
            .execute(
                serde_json::json!({ "act": "OBCA", "topic": "share certificates" }),
                &JobContext::default(),
            )
            .await
            .expect("unknown topics should return advisory text");

        assert!(
            output
                .result
                .as_str()
                .expect("text result")
                .contains("No local compliance topic matched")
        );
    }
}
