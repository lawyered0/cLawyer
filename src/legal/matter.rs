use std::fmt;

use serde::{Deserialize, Serialize};

use crate::config::LegalConfig;
use crate::error::WorkspaceError;
use crate::legal::policy::sanitize_matter_id;
use crate::workspace::Workspace;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterMetadata {
    pub matter_id: String,
    pub client: String,
    pub team: Vec<String>,
    pub confidentiality: String,
    pub adversaries: Vec<String>,
    pub retention: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatterMetadataValidationError {
    Missing { path: String },
    Invalid { path: String, reason: String },
    Storage { path: String, reason: String },
}

impl fmt::Display for MatterMetadataValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { path } => {
                write!(f, "missing required matter metadata at '{}'", path)
            }
            Self::Invalid { path, reason } => write!(f, "{} in '{}'", reason, path),
            Self::Storage { path, reason } => {
                write!(f, "failed to read matter metadata '{}': {}", path, reason)
            }
        }
    }
}

impl MatterMetadata {
    pub fn validate_required_fields(&self) -> Result<(), String> {
        if self.matter_id.trim().is_empty() {
            return Err("matter_id is required".to_string());
        }
        if self.client.trim().is_empty() {
            return Err("client is required".to_string());
        }
        if self.confidentiality.trim().is_empty() {
            return Err("confidentiality is required".to_string());
        }
        if self.retention.trim().is_empty() {
            return Err("retention is required".to_string());
        }
        Ok(())
    }
}

pub fn matter_prefix(config: &LegalConfig, matter_id: &str) -> String {
    let root = config.matter_root.trim_matches('/');
    let id = sanitize_matter_id(matter_id);
    format!("{root}/{id}")
}

pub fn matter_metadata_path(config: &LegalConfig, matter_id: &str) -> String {
    format!("{}/matter.yaml", matter_prefix(config, matter_id))
}

pub fn matter_metadata_path_for_root(matter_root: &str, matter_id: &str) -> String {
    let root = matter_root.trim_matches('/');
    let id = sanitize_matter_id(matter_id);
    format!("{root}/{id}/matter.yaml")
}

pub async fn read_matter_metadata_for_root(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Result<MatterMetadata, MatterMetadataValidationError> {
    let metadata_path = matter_metadata_path_for_root(matter_root, matter_id);
    let doc = workspace
        .read(&metadata_path)
        .await
        .map_err(|err| match err {
            WorkspaceError::DocumentNotFound { .. } => MatterMetadataValidationError::Missing {
                path: metadata_path.clone(),
            },
            other => MatterMetadataValidationError::Storage {
                path: metadata_path.clone(),
                reason: other.to_string(),
            },
        })?;

    let metadata: MatterMetadata =
        serde_yml::from_str(&doc.content).map_err(|e| MatterMetadataValidationError::Invalid {
            path: metadata_path.clone(),
            reason: format!("invalid matter.yaml format: {}", e),
        })?;

    metadata.validate_required_fields().map_err(|reason| {
        MatterMetadataValidationError::Invalid {
            path: metadata_path.clone(),
            reason,
        }
    })?;

    let expected = sanitize_matter_id(matter_id);
    if metadata.matter_id != expected {
        return Err(MatterMetadataValidationError::Invalid {
            path: metadata_path,
            reason: format!(
                "matter.yaml mismatch: expected matter_id '{}', got '{}'",
                expected, metadata.matter_id
            ),
        });
    }

    Ok(metadata)
}

/// Validate `matter.yaml` for the active matter context.
pub async fn validate_active_matter_metadata(
    workspace: &Workspace,
    config: &LegalConfig,
) -> Result<(), String> {
    if !config.enabled {
        return Ok(());
    }

    let matter_id = match config.active_matter.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Ok(()),
    };

    read_matter_metadata_for_root(workspace, &config.matter_root, matter_id)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Seed legal workspace scaffolding if legal mode is enabled.
pub async fn seed_legal_workspace(
    workspace: &Workspace,
    config: &LegalConfig,
) -> Result<(), WorkspaceError> {
    if !config.enabled {
        return Ok(());
    }

    // Seed conflict list template.
    match workspace.read("conflicts.json").await {
        Ok(_) => {}
        Err(WorkspaceError::DocumentNotFound { .. }) => {
            workspace
                .write(
                    "conflicts.json",
                    "[\n  {\n    \"name\": \"Example Adverse Party\",\n    \"aliases\": [\"Example Co\"]\n  }\n]\n",
                )
                .await?;
        }
        Err(e) => return Err(e),
    }

    let root_seeds = [
        (
            "AGENTS.md".to_string(),
            "# cLawyer Legal Guardrails\n\n\
             - Treat all `matters/*` files as confidential by default.\n\
             - Require source citations for factual/legal assertions.\n\
             - If evidence is missing, state `insufficient evidence`.\n\
             - Keep facts and analysis in separate sections.\n\
             - Do not export matter data externally without explicit approval.\n"
                .to_string(),
        ),
        (
            "legal/CITATION_STYLE_GUIDE.md".to_string(),
            "# Citation Style Guide\n\n\
             Use short source references after each supported statement:\n\
             - `[doc:<name> page:<n> section:<heading>]`\n\
             - `[authority:<name> §<section>]`\n\
             - For uncertain support, mark: `insufficient evidence`.\n"
                .to_string(),
        ),
        (
            "legal/CONFIDENTIALITY_NOTES.md".to_string(),
            "# Confidentiality Handling Notes\n\n\
             - Matter files are privileged by default.\n\
             - Redact SSNs, financial account numbers, and government IDs in exports.\n\
             - Confirm approvals before external transmission or publication.\n"
                .to_string(),
        ),
        (
            format!(
                "{}/_template/matter.yaml",
                config.matter_root.trim_matches('/')
            ),
            "matter_id: example-matter\n\
             client: Example Client\n\
             team:\n\
               - Lead Counsel\n\
             confidentiality: attorney-client-privileged\n\
             adversaries:\n\
               - Example Adverse Party\n\
             retention: follow-firm-policy\n"
                .to_string(),
        ),
    ];

    for (path, content) in root_seeds {
        match workspace.read(&path).await {
            Ok(_) => {}
            Err(WorkspaceError::DocumentNotFound { .. }) => {
                workspace.write(&path, &content).await?;
            }
            Err(e) => return Err(e),
        }
    }

    let matter_id = match config.active_matter.as_deref() {
        Some(m) if !m.trim().is_empty() => m,
        _ => return Ok(()),
    };

    let prefix = matter_prefix(config, matter_id);
    let metadata_path = format!("{prefix}/matter.yaml");
    let metadata = MatterMetadata {
        matter_id: sanitize_matter_id(matter_id),
        client: "TBD Client".to_string(),
        team: vec!["Lead Counsel".to_string()],
        confidentiality: "attorney-client-privileged".to_string(),
        adversaries: Vec::new(),
        retention: "follow-firm-policy".to_string(),
    };
    let matter_yaml =
        serde_yml::to_string(&metadata).map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("failed to serialize matter metadata: {}", e),
        })?;

    let seeds = [
        (
            format!("{prefix}/README.md"),
            format!(
                "# Matter {}\n\nThis matter workspace is scoped for confidential legal work.\n\n\
                 Files in this tree are treated as privileged by default.\n",
                sanitize_matter_id(matter_id)
            ),
        ),
        (
            metadata_path.clone(),
            format!(
                "# Matter metadata schema\n# Required: matter_id, client, confidentiality, retention\n{}",
                matter_yaml
            ),
        ),
        (
            format!("{prefix}/templates/research_memo.md"),
            "# Research Memo Template\n\n## Question Presented\n\n## Brief Answer\n\n## Facts\n- [Doc/page]\n\n## Analysis\n\n## Authorities\n- [citation]\n\n## Uncertainty/Risk\n".to_string(),
        ),
        (
            format!("{prefix}/templates/chronology.md"),
            "# Chronology\n\n| Date | Event | Source |\n|---|---|---|\n".to_string(),
        ),
        (
            format!("{prefix}/templates/contract_issues.md"),
            "# Contract Issue List\n\n## Clause\n\n## Risk\n\n## Recommendation\n\n## Source\n".to_string(),
        ),
        (
            format!("{prefix}/templates/discovery_plan.md"),
            "# Discovery Plan\n\n## Custodians\n\n## Data Sources\n\n## Requests\n\n## Objections/Risks\n\n## Source Traceability\n".to_string(),
        ),
        (
            format!("{prefix}/templates/research_synthesis.md"),
            "# Research Synthesis\n\n## Question Presented\n\n## Authorities Reviewed\n\n## Facts (Cited)\n\n## Analysis\n\n## Uncertainty/Risk\n".to_string(),
        ),
        (
            format!("{prefix}/templates/legal_memo.md"),
            "# Legal Memo\n\n## Issue\n\n## Brief Answer\n\n## Facts (Cited)\n\n## Analysis\n\n## Conclusion\n\n## Uncertainty/Risk\n".to_string(),
        ),
    ];

    for (path, content) in seeds {
        match workspace.read(&path).await {
            Ok(_) => {}
            Err(WorkspaceError::DocumentNotFound { .. }) => {
                workspace.write(&path, &content).await?;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(())
}

/// Check conflicts.json for obvious conflict hits in message or active matter.
pub async fn detect_conflict(
    workspace: &Workspace,
    config: &LegalConfig,
    message: &str,
) -> Option<String> {
    if !config.enabled || !config.conflict_check_enabled {
        return None;
    }

    let doc = workspace.read("conflicts.json").await.ok()?;
    let value: serde_json::Value = serde_json::from_str(&doc.content).ok()?;
    let entries = value.as_array()?;

    let message_lc = message.to_ascii_lowercase();
    let active_matter_lc = config
        .active_matter
        .as_ref()
        .map(|m| m.to_ascii_lowercase())
        .unwrap_or_default();

    for entry in entries {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let aliases: Vec<String> = entry
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_ascii_lowercase())
                    .collect()
            })
            .unwrap_or_default();

        let name_lc = name.to_ascii_lowercase();
        if !name_lc.is_empty()
            && (message_lc.contains(&name_lc) || active_matter_lc.contains(&name_lc))
        {
            return Some(name.to_string());
        }

        for alias in aliases {
            if message_lc.contains(&alias) || active_matter_lc.contains(&alias) {
                return Some(name.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{MatterMetadata, matter_metadata_path_for_root};

    #[test]
    fn matter_metadata_requires_core_fields() {
        let missing = MatterMetadata {
            matter_id: "".to_string(),
            client: "".to_string(),
            team: vec![],
            confidentiality: "".to_string(),
            adversaries: vec![],
            retention: "".to_string(),
        };
        assert!(missing.validate_required_fields().is_err());

        let ok = MatterMetadata {
            matter_id: "acme-v-foo".to_string(),
            client: "Acme".to_string(),
            team: vec!["Lead Counsel".to_string()],
            confidentiality: "attorney-client-privileged".to_string(),
            adversaries: vec!["Foo Corp".to_string()],
            retention: "follow-firm-policy".to_string(),
        };
        assert!(ok.validate_required_fields().is_ok());
    }

    #[test]
    fn matter_metadata_path_for_root_normalizes_id() {
        assert_eq!(
            matter_metadata_path_for_root("matters", "Acme v. Foo"),
            "matters/acme-v--foo/matter.yaml"
        );
    }
}
