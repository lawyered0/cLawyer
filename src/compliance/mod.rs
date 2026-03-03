//! Compliance scoring and attestation prompt helpers.
//!
//! This module evaluates live runtime signals against the NIST AI RMF core
//! functions (Govern / Map / Measure / Manage) and provides a deterministic
//! compliance summary for web/API surfaces.

use crate::config::LegalConfig;

/// Three-state compliance status used across functions/checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplianceState {
    Compliant,
    Partial,
    NeedsReview,
}

impl ComplianceState {
    /// Worst-of ordering: compliant < partial < needs_review.
    pub fn worst(self, other: Self) -> Self {
        use ComplianceState::*;
        match (self, other) {
            (NeedsReview, _) | (_, NeedsReview) => NeedsReview,
            (Partial, _) | (_, Partial) => Partial,
            _ => Compliant,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compliant => "compliant",
            Self::Partial => "partial",
            Self::NeedsReview => "needs_review",
        }
    }

    pub fn as_label(self) -> &'static str {
        match self {
            Self::Compliant => "Compliant",
            Self::Partial => "Partial",
            Self::NeedsReview => "Needs Review",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComplianceCheck {
    pub id: &'static str,
    pub label: &'static str,
    pub status: ComplianceState,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct ComplianceFunction {
    pub status: ComplianceState,
    pub checks: Vec<ComplianceCheck>,
}

#[derive(Debug, Clone, Default)]
pub struct ComplianceMetrics {
    pub matters_total: usize,
    pub matters_classified: usize,
    pub tools_total: usize,
    pub audit_events_total: Option<usize>,
    pub audit_info_count: Option<usize>,
    pub audit_warn_count: Option<usize>,
    pub audit_critical_count: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct ComplianceStatus {
    pub overall: ComplianceState,
    pub govern: ComplianceFunction,
    pub map: ComplianceFunction,
    pub measure: ComplianceFunction,
    pub manage: ComplianceFunction,
    pub metrics: ComplianceMetrics,
    pub data_gaps: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ComplianceRuntimeFacts {
    pub llm_backend: String,
    pub auth_token_required: bool,
    pub safety_injection_enabled: bool,
    pub safety_policy_rule_count: usize,
    pub safety_leak_pattern_count: usize,
}

impl Default for ComplianceRuntimeFacts {
    fn default() -> Self {
        Self {
            llm_backend: "unknown".to_string(),
            auth_token_required: true,
            safety_injection_enabled: true,
            safety_policy_rule_count: 0,
            safety_leak_pattern_count: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComplianceInputs {
    pub audit_enabled: bool,
    pub audit_hash_chain: bool,
    pub hardening_max_lockdown: bool,
    pub conflict_check_enabled: bool,
    pub conflict_file_fallback_enabled: bool,
    pub privilege_guard_enabled: bool,
    pub network_deny_by_default: bool,
    pub redaction_pii: bool,
    pub redaction_phi: bool,
    pub redaction_financial: bool,
    pub matters_total: usize,
    pub matters_classified: usize,
    pub tools_total: usize,
    pub audit_events_total: Option<usize>,
    pub audit_info_count: Option<usize>,
    pub audit_warn_count: Option<usize>,
    pub audit_critical_count: Option<usize>,
    pub runtime: ComplianceRuntimeFacts,
    pub data_gaps: Vec<String>,
}

fn function_status(checks: &[ComplianceCheck]) -> ComplianceState {
    checks
        .iter()
        .fold(ComplianceState::Compliant, |acc, check| {
            acc.worst(check.status)
        })
}

fn check(
    id: &'static str,
    label: &'static str,
    status: ComplianceState,
    detail: impl Into<String>,
) -> ComplianceCheck {
    ComplianceCheck {
        id,
        label,
        status,
        detail: detail.into(),
    }
}

pub fn evaluate_nist_rmf(inputs: &ComplianceInputs) -> ComplianceStatus {
    let govern_checks = vec![
        check(
            "audit_enabled",
            "Legal audit logging enabled",
            if inputs.audit_enabled {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!("legal.audit.enabled={}", inputs.audit_enabled),
        ),
        check(
            "audit_hash_chain",
            "Audit hash-chain enabled",
            if inputs.audit_hash_chain {
                ComplianceState::Compliant
            } else {
                ComplianceState::Partial
            },
            format!("legal.audit.hash_chain={}", inputs.audit_hash_chain),
        ),
        check(
            "hardening_profile",
            "Hardening profile is max_lockdown",
            if inputs.hardening_max_lockdown {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            if inputs.hardening_max_lockdown {
                "legal.hardening=max_lockdown".to_string()
            } else {
                "legal.hardening is not max_lockdown".to_string()
            },
        ),
        check(
            "auth_token_required",
            "Gateway requires auth token",
            if inputs.runtime.auth_token_required {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!(
                "gateway.auth_token_required={}",
                inputs.runtime.auth_token_required
            ),
        ),
    ];

    let coverage_status = if inputs.matters_total == 0 {
        ComplianceState::Partial
    } else if inputs.matters_classified == inputs.matters_total {
        ComplianceState::Compliant
    } else if inputs.matters_classified == 0 {
        ComplianceState::NeedsReview
    } else {
        ComplianceState::Partial
    };
    let map_checks = vec![
        check(
            "matter_classification_coverage",
            "Matter confidentiality/retention coverage",
            coverage_status,
            if inputs.matters_total == 0 {
                "No matters present; classification coverage cannot be fully assessed".to_string()
            } else {
                format!(
                    "{} of {} matters include confidentiality+retention",
                    inputs.matters_classified, inputs.matters_total
                )
            },
        ),
        check(
            "conflict_check_enabled",
            "Conflict checking enabled",
            if inputs.conflict_check_enabled {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!(
                "legal.conflict_check_enabled={}",
                inputs.conflict_check_enabled
            ),
        ),
        check(
            "tool_inventory_present",
            "AI tool inventory available",
            if inputs.tools_total > 0 {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!("registered_tools={}", inputs.tools_total),
        ),
    ];

    let redaction_enabled_count = [
        inputs.redaction_pii,
        inputs.redaction_phi,
        inputs.redaction_financial,
    ]
    .into_iter()
    .filter(|enabled| *enabled)
    .count();
    let measure_checks = vec![
        check(
            "redaction_classes_active",
            "PII/PHI/financial redaction classes active",
            if redaction_enabled_count == 3 {
                ComplianceState::Compliant
            } else if redaction_enabled_count > 0 {
                ComplianceState::Partial
            } else {
                ComplianceState::NeedsReview
            },
            format!(
                "pii={}, phi={}, financial={}",
                inputs.redaction_pii, inputs.redaction_phi, inputs.redaction_financial
            ),
        ),
        check(
            "injection_detection_enabled",
            "Prompt-injection detection enabled",
            if inputs.runtime.safety_injection_enabled {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!(
                "safety.injection_check_enabled={}",
                inputs.runtime.safety_injection_enabled
            ),
        ),
        check(
            "network_deny_by_default",
            "Network deny-by-default enabled",
            if inputs.network_deny_by_default {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!(
                "legal.network.deny_by_default={}",
                inputs.network_deny_by_default
            ),
        ),
        check(
            "audit_activity_observed",
            "Audit activity observed",
            if inputs.audit_events_total.is_some_and(|count| count > 0) {
                ComplianceState::Compliant
            } else {
                ComplianceState::Partial
            },
            match inputs.audit_events_total {
                Some(count) => format!("audit_events_logged={count}"),
                None => {
                    "Audit event count unavailable (database unavailable or read error)".to_string()
                }
            },
        ),
    ];

    let backend = inputs.runtime.llm_backend.to_ascii_lowercase();
    let measure_backend_local_or_tee = matches!(backend.as_str(), "ollama" | "tinfoil");
    let manage_checks = vec![
        check(
            "privilege_guard_enabled",
            "Privilege guard enabled",
            if inputs.privilege_guard_enabled {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            format!("legal.privilege_guard={}", inputs.privilege_guard_enabled),
        ),
        check(
            "max_lockdown_active",
            "Max-lockdown profile active",
            if inputs.hardening_max_lockdown {
                ComplianceState::Compliant
            } else {
                ComplianceState::NeedsReview
            },
            if inputs.hardening_max_lockdown {
                "legal.hardening=max_lockdown".to_string()
            } else {
                "legal.hardening is not max_lockdown".to_string()
            },
        ),
        check(
            "conflict_fallback_enabled",
            "Conflict fallback enabled",
            if inputs.conflict_file_fallback_enabled {
                ComplianceState::Compliant
            } else {
                ComplianceState::Partial
            },
            format!(
                "legal.conflict_file_fallback_enabled={}",
                inputs.conflict_file_fallback_enabled
            ),
        ),
        check(
            "llm_local_or_tee",
            "LLM backend is local or TEE",
            if measure_backend_local_or_tee {
                ComplianceState::Compliant
            } else {
                ComplianceState::Partial
            },
            format!("llm_backend={}", inputs.runtime.llm_backend),
        ),
    ];

    let govern = ComplianceFunction {
        status: function_status(&govern_checks),
        checks: govern_checks,
    };
    let map = ComplianceFunction {
        status: function_status(&map_checks),
        checks: map_checks,
    };
    let measure = ComplianceFunction {
        status: function_status(&measure_checks),
        checks: measure_checks,
    };
    let manage = ComplianceFunction {
        status: function_status(&manage_checks),
        checks: manage_checks,
    };

    let overall = govern
        .status
        .worst(map.status)
        .worst(measure.status)
        .worst(manage.status);

    ComplianceStatus {
        overall,
        govern,
        map,
        measure,
        manage,
        metrics: ComplianceMetrics {
            matters_total: inputs.matters_total,
            matters_classified: inputs.matters_classified,
            tools_total: inputs.tools_total,
            audit_events_total: inputs.audit_events_total,
            audit_info_count: inputs.audit_info_count,
            audit_warn_count: inputs.audit_warn_count,
            audit_critical_count: inputs.audit_critical_count,
        },
        data_gaps: inputs.data_gaps.clone(),
    }
}

/// Build the LLM prompt used for compliance attestation letters.
pub fn build_attestation_prompt(
    framework: &str,
    firm_name: Option<&str>,
    generated_at: &str,
    legal: &LegalConfig,
    status: &ComplianceStatus,
    llm_model: &str,
) -> String {
    let firm = firm_name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("This Firm");

    format!(
        "Generate a factual compliance attestation letter in Markdown.\n\
         \n\
         Constraints:\n\
         - Report only the supplied runtime/configuration facts.\n\
         - Do not invent controls, policies, or certifications.\n\
         - Do not provide legal advice.\n\
         - If a value is unavailable, say \"Unavailable\".\n\
         \n\
         Audience: legal operations and client security reviewers.\n\
         Framework: {framework}\n\
         Firm Name: {firm}\n\
         Generated At (UTC): {generated_at}\n\
         LLM Model: {llm_model}\n\
         \n\
         Runtime Facts:\n\
         - legal.enabled: {legal_enabled}\n\
         - legal.jurisdiction: {jurisdiction}\n\
         - legal.hardening: {hardening}\n\
         - legal.require_matter_context: {require_matter}\n\
         - legal.citation_required: {citation_required}\n\
         - legal.privilege_guard: {privilege_guard}\n\
         - legal.conflict_check_enabled: {conflict_check}\n\
         - legal.conflict_file_fallback_enabled: {conflict_fallback}\n\
         - legal.network.deny_by_default: {deny_network}\n\
         - legal.audit.enabled: {audit_enabled}\n\
         - legal.audit.hash_chain: {audit_hash_chain}\n\
         - legal.redaction.pii/phi/financial/government_id: {pii}/{phi}/{financial}/{gov_id}\n\
         \n\
         NIST AI RMF Status:\n\
         - overall: {overall}\n\
         - govern: {govern}\n\
         - map: {map}\n\
         - measure: {measure}\n\
         - manage: {manage}\n\
         \n\
         Metrics:\n\
         - matters_total: {matters_total}\n\
         - matters_classified: {matters_classified}\n\
         - tools_total: {tools_total}\n\
         - audit_events_total: {audit_events_total}\n\
         - audit_info_count: {audit_info_count}\n\
         - audit_warn_count: {audit_warn_count}\n\
         - audit_critical_count: {audit_critical_count}\n\
         \n\
         Data gaps:\n\
         {data_gaps}\n\
         \n\
         Required output sections:\n\
         1. Executive Summary\n\
         2. Scope and Method\n\
         3. Control Posture by NIST Function (Govern/Map/Measure/Manage)\n\
         4. Runtime Configuration Evidence\n\
         5. Data Gaps and Limitations\n\
         6. Attestation Statement\n\
         \n\
         Keep claims factual and concise.",
        legal_enabled = legal.enabled,
        jurisdiction = legal.jurisdiction,
        hardening = legal.hardening.as_str(),
        require_matter = legal.require_matter_context,
        citation_required = legal.citation_required,
        privilege_guard = legal.privilege_guard,
        conflict_check = legal.conflict_check_enabled,
        conflict_fallback = legal.conflict_file_fallback_enabled,
        deny_network = legal.network.deny_by_default,
        audit_enabled = legal.audit.enabled,
        audit_hash_chain = legal.audit.hash_chain,
        pii = legal.redaction.pii,
        phi = legal.redaction.phi,
        financial = legal.redaction.financial,
        gov_id = legal.redaction.government_id,
        overall = status.overall.as_str(),
        govern = status.govern.status.as_str(),
        map = status.map.status.as_str(),
        measure = status.measure.status.as_str(),
        manage = status.manage.status.as_str(),
        matters_total = status.metrics.matters_total,
        matters_classified = status.metrics.matters_classified,
        tools_total = status.metrics.tools_total,
        audit_events_total = status
            .metrics
            .audit_events_total
            .map(|v| v.to_string())
            .unwrap_or_else(|| "Unavailable".to_string()),
        audit_info_count = status
            .metrics
            .audit_info_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "Unavailable".to_string()),
        audit_warn_count = status
            .metrics
            .audit_warn_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "Unavailable".to_string()),
        audit_critical_count = status
            .metrics
            .audit_critical_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "Unavailable".to_string()),
        data_gaps = if status.data_gaps.is_empty() {
            "- None".to_string()
        } else {
            status
                .data_gaps
                .iter()
                .map(|gap| format!("- {gap}"))
                .collect::<Vec<_>>()
                .join("\n")
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_inputs() -> ComplianceInputs {
        ComplianceInputs {
            audit_enabled: true,
            audit_hash_chain: true,
            hardening_max_lockdown: true,
            conflict_check_enabled: true,
            conflict_file_fallback_enabled: true,
            privilege_guard_enabled: true,
            network_deny_by_default: true,
            redaction_pii: true,
            redaction_phi: true,
            redaction_financial: true,
            matters_total: 2,
            matters_classified: 2,
            tools_total: 3,
            audit_events_total: Some(5),
            audit_info_count: Some(4),
            audit_warn_count: Some(1),
            audit_critical_count: Some(0),
            runtime: ComplianceRuntimeFacts {
                llm_backend: "ollama".to_string(),
                auth_token_required: true,
                safety_injection_enabled: true,
                safety_policy_rule_count: 6,
                safety_leak_pattern_count: 12,
            },
            data_gaps: Vec::new(),
        }
    }

    #[test]
    fn evaluate_nist_marks_non_lockdown_manage_as_needs_review() {
        let mut inputs = base_inputs();
        inputs.hardening_max_lockdown = false;
        let result = evaluate_nist_rmf(&inputs);
        assert_eq!(result.manage.status, ComplianceState::NeedsReview);
        assert_eq!(result.overall, ComplianceState::NeedsReview);
    }

    #[test]
    fn evaluate_nist_marks_empty_tool_inventory_as_needs_review() {
        let mut inputs = base_inputs();
        inputs.tools_total = 0;
        let result = evaluate_nist_rmf(&inputs);
        assert_eq!(result.map.status, ComplianceState::NeedsReview);
    }

    #[test]
    fn evaluate_nist_treats_no_matters_as_partial_not_fail() {
        let mut inputs = base_inputs();
        inputs.matters_total = 0;
        inputs.matters_classified = 0;
        let result = evaluate_nist_rmf(&inputs);
        let coverage = result
            .map
            .checks
            .iter()
            .find(|check| check.id == "matter_classification_coverage")
            .expect("coverage check");
        assert_eq!(coverage.status, ComplianceState::Partial);
    }
}
