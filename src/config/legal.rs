use std::path::{Component, PathBuf};

use crate::config::helpers::{optional_env, parse_bool_env, parse_string_env};
use crate::error::ConfigError;
use crate::settings::Settings;

/// Hardening profile for legal-mode enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegalHardeningProfile {
    Standard,
    MaxLockdown,
}

impl LegalHardeningProfile {
    fn from_str(value: &str) -> Result<Self, ConfigError> {
        match value.to_ascii_lowercase().as_str() {
            "standard" => Ok(Self::Standard),
            "max_lockdown" | "max-lockdown" => Ok(Self::MaxLockdown),
            other => Err(ConfigError::InvalidValue {
                key: "LEGAL_HARDENING".to_string(),
                message: format!("unsupported profile '{other}'"),
            }),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::MaxLockdown => "max_lockdown",
        }
    }
}

/// Legal network controls.
#[derive(Debug, Clone)]
pub struct LegalNetworkConfig {
    pub deny_by_default: bool,
    pub allowed_domains: Vec<String>,
}

/// Legal audit controls.
#[derive(Debug, Clone)]
pub struct LegalAuditConfig {
    pub enabled: bool,
    pub path: PathBuf,
    pub hash_chain: bool,
}

/// Legal redaction controls.
#[derive(Debug, Clone)]
pub struct LegalRedactionConfig {
    pub pii: bool,
    pub phi: bool,
    pub financial: bool,
    pub government_id: bool,
}

/// Legal workflow profile and policy controls.
#[derive(Debug, Clone)]
pub struct LegalConfig {
    pub enabled: bool,
    pub jurisdiction: String,
    pub hardening: LegalHardeningProfile,
    pub require_matter_context: bool,
    pub citation_required: bool,
    pub matter_root: String,
    pub active_matter: Option<String>,
    pub privilege_guard: bool,
    pub conflict_check_enabled: bool,
    pub network: LegalNetworkConfig,
    pub audit: LegalAuditConfig,
    pub redaction: LegalRedactionConfig,
}

fn parse_domains_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .collect()
}

fn sanitize_active_matter(raw: Option<String>) -> Option<String> {
    raw.and_then(|value| {
        let sanitized = crate::legal::policy::sanitize_matter_id(&value);
        if sanitized.is_empty() {
            None
        } else {
            Some(sanitized)
        }
    })
}

fn validate_audit_path(raw: &str) -> Result<PathBuf, ConfigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_AUDIT_PATH".to_string(),
            message: "audit log path must not be empty".to_string(),
        });
    }

    let raw_path = PathBuf::from(trimmed);
    if raw_path.is_absolute() {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_AUDIT_PATH".to_string(),
            message: "audit log path must be relative to the workspace".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in raw_path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ConfigError::InvalidValue {
                    key: "LEGAL_AUDIT_PATH".to_string(),
                    message: "audit log path must not contain '..' components".to_string(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ConfigError::InvalidValue {
                    key: "LEGAL_AUDIT_PATH".to_string(),
                    message: "audit log path must be relative to the workspace".to_string(),
                });
            }
        }
    }

    if normalized.components().count() < 2 || !normalized.starts_with("logs") {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_AUDIT_PATH".to_string(),
            message: "audit log path must be under 'logs/' and include a filename".to_string(),
        });
    }

    Ok(normalized)
}

fn validate_matter_root(raw: &str) -> Result<String, ConfigError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_MATTER_ROOT".to_string(),
            message: "matter root must not be empty".to_string(),
        });
    }

    // Parse BEFORE stripping any leading slash so is_absolute() works correctly.
    let raw_path = PathBuf::from(trimmed);
    if raw_path.is_absolute() {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_MATTER_ROOT".to_string(),
            message: "matter root must be relative to the workspace".to_string(),
        });
    }

    let mut normalized = PathBuf::new();
    for component in raw_path.components() {
        match component {
            Component::Normal(segment) => normalized.push(segment),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ConfigError::InvalidValue {
                    key: "LEGAL_MATTER_ROOT".to_string(),
                    message: "matter root must not contain '..' components".to_string(),
                });
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(ConfigError::InvalidValue {
                    key: "LEGAL_MATTER_ROOT".to_string(),
                    message: "matter root must be relative to the workspace".to_string(),
                });
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ConfigError::InvalidValue {
            key: "LEGAL_MATTER_ROOT".to_string(),
            message: "matter root must not be empty".to_string(),
        });
    }

    normalized
        .to_str()
        .ok_or_else(|| ConfigError::InvalidValue {
            key: "LEGAL_MATTER_ROOT".to_string(),
            message: "matter root contains non-UTF-8 characters".to_string(),
        })
        .map(|s| s.to_string())
}

impl LegalConfig {
    pub(crate) fn resolve(settings: &Settings) -> Result<Self, ConfigError> {
        let hardening_raw = parse_string_env("LEGAL_HARDENING", settings.legal.hardening.clone())?;
        let hardening = LegalHardeningProfile::from_str(&hardening_raw)?;
        let active_matter = sanitize_active_matter(
            optional_env("LEGAL_MATTER")?
                .or_else(|| optional_env("MATTER_ID").ok().flatten())
                .or_else(|| settings.legal.active_matter.clone()),
        );

        let allowed_domains = match optional_env("LEGAL_NETWORK_ALLOWED_DOMAINS")? {
            Some(raw) => parse_domains_csv(&raw),
            None => settings
                .legal
                .network
                .allowed_domains
                .iter()
                .map(|d| d.to_ascii_lowercase())
                .collect(),
        };

        Ok(Self {
            enabled: parse_bool_env("LEGAL_ENABLED", settings.legal.enabled)?,
            jurisdiction: parse_string_env(
                "LEGAL_JURISDICTION",
                settings.legal.jurisdiction.clone(),
            )?,
            hardening,
            require_matter_context: parse_bool_env(
                "LEGAL_REQUIRE_MATTER_CONTEXT",
                settings.legal.require_matter_context,
            )?,
            citation_required: parse_bool_env(
                "LEGAL_CITATION_REQUIRED",
                settings.legal.citation_required,
            )?,
            matter_root: {
                let raw =
                    parse_string_env("LEGAL_MATTER_ROOT", settings.legal.matter_root.clone())?;
                validate_matter_root(&raw)?
            },
            active_matter,
            privilege_guard: parse_bool_env(
                "LEGAL_PRIVILEGE_GUARD",
                settings.legal.privilege_guard,
            )?,
            conflict_check_enabled: parse_bool_env(
                "LEGAL_CONFLICT_CHECK_ENABLED",
                settings.legal.conflict_check_enabled,
            )?,
            network: LegalNetworkConfig {
                deny_by_default: parse_bool_env(
                    "LEGAL_NETWORK_DENY_BY_DEFAULT",
                    settings.legal.network.deny_by_default,
                )?,
                allowed_domains,
            },
            audit: LegalAuditConfig {
                enabled: parse_bool_env("LEGAL_AUDIT_ENABLED", settings.legal.audit.enabled)?,
                path: {
                    let raw =
                        parse_string_env("LEGAL_AUDIT_PATH", settings.legal.audit.path.clone())?;
                    validate_audit_path(&raw)?
                },
                hash_chain: parse_bool_env(
                    "LEGAL_AUDIT_HASH_CHAIN",
                    settings.legal.audit.hash_chain,
                )?,
            },
            redaction: LegalRedactionConfig {
                pii: parse_bool_env("LEGAL_REDACTION_PII", settings.legal.redaction.pii)?,
                phi: parse_bool_env("LEGAL_REDACTION_PHI", settings.legal.redaction.phi)?,
                financial: parse_bool_env(
                    "LEGAL_REDACTION_FINANCIAL",
                    settings.legal.redaction.financial,
                )?,
                government_id: parse_bool_env(
                    "LEGAL_REDACTION_GOVERNMENT_ID",
                    settings.legal.redaction.government_id,
                )?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::error::ConfigError;
    use crate::settings::Settings;

    #[test]
    fn legal_resolve_uses_secure_defaults() {
        let settings = Settings::default();
        let config = super::LegalConfig::resolve(&settings).expect("legal config");

        assert!(config.enabled);
        assert_eq!(config.jurisdiction, "us-general");
        assert_eq!(config.hardening.as_str(), "max_lockdown");
        assert!(config.require_matter_context);
        assert!(config.citation_required);
        assert_eq!(config.matter_root, "matters");
        assert!(config.network.deny_by_default);
        assert!(config.audit.enabled);
        assert!(config.audit.hash_chain);
    }

    #[test]
    fn legal_resolve_sanitizes_active_matter_from_settings() {
        let mut settings = Settings::default();
        settings.legal.active_matter = Some(" Acme v. Foo/2026 ".to_string());

        let config = super::LegalConfig::resolve(&settings).expect("legal config");
        assert_eq!(config.active_matter.as_deref(), Some("acme-v--foo-2026"));
    }

    #[test]
    fn legal_resolve_drops_empty_active_matter_after_sanitization() {
        let mut settings = Settings::default();
        settings.legal.active_matter = Some("   !!!   ".to_string());

        let config = super::LegalConfig::resolve(&settings).expect("legal config");
        assert_eq!(config.active_matter, None);
    }

    #[test]
    fn validate_audit_path_accepts_normalized_logs_subpaths() {
        let path = super::validate_audit_path("./logs//cases/./audit.jsonl/")
            .expect("path should be accepted");
        assert_eq!(path, PathBuf::from("logs/cases/audit.jsonl"));
    }

    #[test]
    fn validate_audit_path_rejects_parent_dir_traversal() {
        let err = super::validate_audit_path("logs/../audit.jsonl").expect_err("must reject '..'");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_AUDIT_PATH");
        assert!(message.contains(".."), "unexpected message: {message}");
    }

    #[test]
    fn validate_audit_path_rejects_absolute_paths() {
        let absolute = if cfg!(windows) {
            r"C:\tmp\audit.jsonl"
        } else {
            "/tmp/audit.jsonl"
        };
        let err =
            super::validate_audit_path(absolute).expect_err("absolute paths must be rejected");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_AUDIT_PATH");
        assert!(
            message.contains("relative to the workspace"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn validate_audit_path_rejects_paths_outside_logs_allowlist() {
        let err =
            super::validate_audit_path("tmp/legal_audit.jsonl").expect_err("must stay under logs/");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_AUDIT_PATH");
        assert!(
            message.contains("under 'logs/'"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn validate_matter_root_accepts_simple_relative_path() {
        assert_eq!(
            super::validate_matter_root("matters").expect("valid"),
            "matters"
        );
    }

    #[test]
    fn validate_matter_root_accepts_nested_relative_path() {
        assert_eq!(
            super::validate_matter_root("legal/matters").expect("valid"),
            "legal/matters"
        );
    }

    #[test]
    fn validate_matter_root_normalizes_cur_dir_and_trailing_slashes() {
        assert_eq!(
            super::validate_matter_root("./matters/./files/").expect("valid"),
            "matters/files"
        );
    }

    #[test]
    fn validate_matter_root_rejects_parent_dir_traversal() {
        let err = super::validate_matter_root("../matters").expect_err("must reject '..'");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_MATTER_ROOT");
        assert!(message.contains(".."), "unexpected message: {message}");
    }

    #[test]
    fn validate_matter_root_rejects_embedded_traversal() {
        let err = super::validate_matter_root("matters/../../etc").expect_err("must reject '..'");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_MATTER_ROOT");
        assert!(message.contains(".."), "unexpected message: {message}");
    }

    #[test]
    fn validate_matter_root_rejects_absolute_paths() {
        let absolute = if cfg!(windows) {
            r"C:\matters"
        } else {
            "/tmp/matters"
        };
        let err =
            super::validate_matter_root(absolute).expect_err("absolute paths must be rejected");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_MATTER_ROOT");
        assert!(
            message.contains("relative to the workspace"),
            "unexpected message: {message}"
        );
    }

    #[test]
    fn validate_matter_root_rejects_empty() {
        let err = super::validate_matter_root("   ").expect_err("empty must be rejected");
        let ConfigError::InvalidValue { key, message } = err else {
            panic!("expected InvalidValue");
        };
        assert_eq!(key, "LEGAL_MATTER_ROOT");
        assert!(message.contains("empty"), "unexpected message: {message}");
    }
}
