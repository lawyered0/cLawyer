use crate::config::{LegalConfig, LegalHardeningProfile};

const MAX_LOCKDOWN_SIDE_EFFECT_TOOLS: &[&str] = &[
    "http",
    "shell",
    "write_file",
    "apply_patch",
    "memory_write",
    "create_job",
    "cancel_job",
    "tool_install",
    "tool_activate",
    "tool_remove",
    "skill_install",
    "skill_remove",
    "routine_create",
    "routine_update",
    "routine_delete",
];
const PRIVILEGE_GUARD_EGRESS_TOOLS: &[&str] = &["http", "shell", "create_job"];
const PRIVILEGE_GUARD_CONFIDENTIAL_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "list_dir",
    "apply_patch",
    "memory_read",
    "memory_write",
    "memory_search",
    "memory_tree",
];

/// True when legal hardening is in max-lockdown mode.
pub fn is_max_lockdown(config: &LegalConfig) -> bool {
    config.enabled && config.hardening == LegalHardeningProfile::MaxLockdown
}

/// Whether a tool should always require explicit approval in legal max-lockdown mode.
pub fn requires_explicit_approval(config: &LegalConfig, tool_name: &str) -> bool {
    if !config.enabled {
        return false;
    }

    if is_max_lockdown(config) && MAX_LOCKDOWN_SIDE_EFFECT_TOOLS.contains(&tool_name) {
        return true;
    }

    let active_matter_set = config
        .active_matter
        .as_deref()
        .is_some_and(|m| !m.trim().is_empty());
    if config.privilege_guard
        && active_matter_set
        && (PRIVILEGE_GUARD_EGRESS_TOOLS.contains(&tool_name)
            || PRIVILEGE_GUARD_CONFIDENTIAL_TOOLS.contains(&tool_name))
    {
        return true;
    }

    false
}

/// Normalize a domain for allowlist comparisons.
pub fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// Check if a target host is allowed by legal network policy.
pub fn is_network_domain_allowed(config: &LegalConfig, host: &str) -> bool {
    if !config.enabled || !config.network.deny_by_default {
        return true;
    }

    let host = normalize_domain(host);
    if host.is_empty() {
        return false;
    }

    config.network.allowed_domains.iter().any(|raw| {
        let allowed = normalize_domain(raw);
        host == allowed || host.ends_with(&format!(".{allowed}"))
    })
}

/// Keep matter IDs filesystem-safe and deterministic.
pub fn sanitize_matter_id(matter_id: &str) -> String {
    matter_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Basic heuristic for identifying non-trivial legal tasks.
pub fn is_non_trivial_request(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.len() >= 32 {
        return true;
    }

    let words = trimmed.split_whitespace().count();
    if words > 5 {
        return true;
    }

    let lower = trimmed.to_ascii_lowercase();
    [
        "contract",
        "motion",
        "brief",
        "complaint",
        "citation",
        "deposition",
        "discovery",
        "research",
        "chronology",
        "evidence",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// Heuristic citation check for generated responses.
pub fn response_has_citation_markers(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    lower.contains("source:")
        || lower.contains("sources:")
        || lower.contains("citation:")
        || lower.contains("citations:")
        || lower.contains("[doc")
        || lower.contains("(doc")
        || lower.contains("[§")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::{LegalConfig, LegalHardeningProfile, LegalNetworkConfig};

    fn legal_for_test() -> LegalConfig {
        LegalConfig {
            enabled: true,
            jurisdiction: "us-general".to_string(),
            hardening: LegalHardeningProfile::MaxLockdown,
            require_matter_context: true,
            citation_required: true,
            matter_root: "matters".to_string(),
            active_matter: Some("demo-matter".to_string()),
            privilege_guard: true,
            conflict_check_enabled: true,
            network: LegalNetworkConfig {
                deny_by_default: true,
                allowed_domains: vec!["example.com".to_string()],
            },
            audit: crate::config::LegalAuditConfig {
                enabled: true,
                path: "logs/legal_audit.jsonl".into(),
                hash_chain: true,
            },
            redaction: crate::config::LegalRedactionConfig {
                pii: true,
                phi: true,
                financial: true,
                government_id: true,
            },
        }
    }

    #[test]
    fn allowlist_domain_check_supports_suffixes() {
        let cfg = legal_for_test();
        assert!(is_network_domain_allowed(&cfg, "example.com"));
        assert!(is_network_domain_allowed(&cfg, "api.example.com"));
        assert!(!is_network_domain_allowed(&cfg, "example.org"));
    }

    #[test]
    fn max_lockdown_still_forces_side_effect_approval() {
        let mut cfg = legal_for_test();
        cfg.privilege_guard = false;
        cfg.active_matter = None;
        assert!(requires_explicit_approval(&cfg, "write_file"));
        assert!(!requires_explicit_approval(&cfg, "memory_read"));
    }

    #[test]
    fn privilege_guard_forces_approval_for_sensitive_tools_with_active_matter() {
        let mut cfg = legal_for_test();
        cfg.hardening = LegalHardeningProfile::Standard;
        cfg.active_matter = Some("demo".to_string());
        cfg.privilege_guard = true;

        assert!(requires_explicit_approval(&cfg, "http"));
        assert!(requires_explicit_approval(&cfg, "read_file"));
        assert!(requires_explicit_approval(&cfg, "memory_write"));
        assert!(!requires_explicit_approval(&cfg, "echo"));
    }

    #[test]
    fn privilege_guard_does_not_force_without_active_matter() {
        let mut cfg = legal_for_test();
        cfg.hardening = LegalHardeningProfile::Standard;
        cfg.active_matter = None;
        cfg.privilege_guard = true;

        assert!(!requires_explicit_approval(&cfg, "shell"));
        assert!(!requires_explicit_approval(&cfg, "read_file"));
    }

    #[test]
    fn citation_heuristic_detects_common_markers() {
        assert!(response_has_citation_markers("Source: Contract §2.1"));
        assert!(response_has_citation_markers("See [doc 4 page 2]"));
        assert!(!response_has_citation_markers(
            "This paragraph has no supporting references."
        ));
    }

    #[test]
    fn sanitize_matter_id_removes_unsafe_chars() {
        assert_eq!(sanitize_matter_id(" Acme v. Foo/2026 "), "acme-v--foo-2026");
    }
}
