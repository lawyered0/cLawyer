//! Legal config, audit, and path policy helpers for web handlers.

use std::path::{Component as FsComponent, Path as FsPath};

use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::channels::web::state::GatewayState;
use crate::db::AuditSeverity;
use crate::workspace::paths;

pub(crate) const MATTER_ROOT: &str = "matters";
/// Settings key used to persist the active matter ID.
pub(crate) const MATTER_ACTIVE_SETTING: &str = "legal.active_matter";
/// Maximum number of party names accepted by intake conflict endpoints.
pub(crate) const MAX_INTAKE_CONFLICT_PARTIES: usize = 64;
/// Maximum reminder offsets accepted for a single deadline.
pub(crate) const MAX_DEADLINE_REMINDERS: usize = 16;
/// Maximum allowed reminder offset in days.
pub(crate) const MAX_DEADLINE_REMINDER_DAYS: i32 = 3650;
/// Maximum allowed body text length for `/api/matters/conflicts/check`.
pub(crate) const MAX_CONFLICT_CHECK_TEXT_LEN: usize = 32 * 1024;
/// Default invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_DEFAULT_LIMIT: usize = 25;
/// Maximum invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_MAX_LIMIT: usize = 100;

/// Identity files that must not be overwritten through web memory-write APIs.
const PROTECTED_IDENTITY_FILES: &[&str] =
    &[paths::IDENTITY, paths::SOUL, paths::AGENTS, paths::USER];

pub(crate) fn legal_config_for_gateway(
    state: &GatewayState,
) -> Result<crate::config::LegalConfig, crate::error::ConfigError> {
    if let Some(config) = state.legal_config.clone() {
        return Ok(config);
    }
    crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
}

pub(crate) fn legal_config_for_gateway_or_500(
    state: &GatewayState,
) -> Result<crate::config::LegalConfig, (StatusCode, String)> {
    legal_config_for_gateway(state).map_err(|err| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to resolve legal config: {err}"),
        )
    })
}

pub(crate) fn matter_root_for_gateway(state: &GatewayState) -> String {
    let configured = match legal_config_for_gateway(state) {
        Ok(config) => config.matter_root,
        Err(err) => {
            tracing::error!(
                "Failed to resolve legal config for matter root; using default '{}': {}",
                MATTER_ROOT,
                err
            );
            MATTER_ROOT.to_string()
        }
    };
    let normalized = configured.trim_matches('/');
    if normalized.is_empty() {
        MATTER_ROOT.to_string()
    } else {
        normalized.to_string()
    }
}

pub(crate) fn matter_prefix_for_gateway(state: &GatewayState, matter_id: &str) -> String {
    format!("{}/{matter_id}", matter_root_for_gateway(state))
}

pub(crate) fn matter_metadata_path_for_gateway(state: &GatewayState, matter_id: &str) -> String {
    format!(
        "{}/matter.yaml",
        matter_prefix_for_gateway(state, matter_id)
    )
}

/// Normalize user-supplied memory paths for policy checks.
///
/// This mirrors workspace normalization semantics that strip leading/trailing
/// slashes, collapse duplicate separators, and ignore `.` segments.
/// `..` segments are preserved and rejected separately by traversal guards.
fn normalize_policy_path(path: &str) -> String {
    let mut parts = Vec::new();
    for component in FsPath::new(path.trim()).components() {
        match component {
            FsComponent::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            FsComponent::ParentDir => parts.push("..".to_string()),
            FsComponent::CurDir | FsComponent::RootDir | FsComponent::Prefix(_) => {}
        }
    }
    parts.join("/")
}

fn is_protected_identity_path(path: &str) -> bool {
    let normalized = normalize_policy_path(path);
    PROTECTED_IDENTITY_FILES
        .iter()
        .any(|protected| normalized.eq_ignore_ascii_case(protected))
}

pub(crate) async fn resolve_memory_write_path_for_gateway(
    state: &GatewayState,
    requested_path: &str,
) -> Result<String, (StatusCode, String)> {
    let normalized = normalize_policy_path(requested_path);
    if normalized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Path is empty after normalization".to_string(),
        ));
    }

    if is_protected_identity_path(&normalized) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "Path '{}' is protected from tool/web writes",
                requested_path
            ),
        ));
    }

    let legal = legal_config_for_gateway_or_500(state)?;
    let resolved_path = if legal.enabled && legal.require_matter_context {
        let active_matter = if let Some(store) = state.store.as_ref() {
            match store
                .get_setting(&state.user_id, MATTER_ACTIVE_SETTING)
                .await
            {
                Ok(value) => value
                    .and_then(|raw| raw.as_str().map(str::to_owned))
                    .and_then(|raw| crate::legal::policy::sanitize_optional_matter_id(&raw)),
                Err(err) => {
                    tracing::warn!(
                        "Failed to load active matter setting for memory write policy: {}",
                        err
                    );
                    None
                }
            }
        } else {
            None
        };
        let matter_id = active_matter
            .or_else(|| legal.active_matter.clone())
            .ok_or((
                StatusCode::FORBIDDEN,
                "No active matter selected. Set an active matter before writing files.".to_string(),
            ))?;
        let matter_root = matter_root_for_gateway(state);
        let matter_prefix = format!("{matter_root}/{matter_id}");
        let matter_root_prefix = format!("{matter_root}/");

        if normalized == matter_prefix || normalized.starts_with(&format!("{matter_prefix}/")) {
            normalized
        } else if normalized == matter_root || normalized.starts_with(&matter_root_prefix) {
            return Err((
                StatusCode::FORBIDDEN,
                format!(
                    "Path '{}' is outside active matter scope '{}'",
                    requested_path, matter_prefix
                ),
            ));
        } else {
            format!("{matter_prefix}/{normalized}")
        }
    } else {
        normalized
    };

    if FsPath::new(&resolved_path)
        .components()
        .any(|component| component == FsComponent::ParentDir)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Path '{}' contains directory traversal sequences",
                requested_path
            ),
        ));
    }

    if is_protected_identity_path(&resolved_path) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "Path '{}' resolves to a protected identity file",
                requested_path
            ),
        ));
    }

    Ok(resolved_path)
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct LegalAuditQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
    pub(crate) event_type: Option<String>,
    pub(crate) matter_id: Option<String>,
    pub(crate) severity: Option<String>,
    pub(crate) since: Option<String>,
    pub(crate) until: Option<String>,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
}

pub(crate) fn parse_utc_query_ts(
    field_name: &str,
    raw: Option<&str>,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid RFC3339 timestamp", field_name),
        )
    })?;
    Ok(Some(parsed.with_timezone(&Utc)))
}

pub(crate) fn parse_audit_severity_query(
    raw: Option<&str>,
) -> Result<Option<AuditSeverity>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "info" => Ok(Some(AuditSeverity::Info)),
        "warn" => Ok(Some(AuditSeverity::Warn)),
        "critical" => Ok(Some(AuditSeverity::Critical)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'severity' must be one of: info, warn, critical".to_string(),
        )),
    }
}

pub(crate) async fn record_legal_audit_event(
    state: &GatewayState,
    event_type: &str,
    actor: &str,
    matter_id: Option<&str>,
    severity: AuditSeverity,
    details: serde_json::Value,
) {
    if let Some(store) = state.store.as_ref() {
        crate::legal::audit::record_with_db(
            event_type,
            actor,
            matter_id,
            severity,
            details,
            store.as_ref(),
            &state.user_id,
        )
        .await;
    } else {
        crate::legal::audit::record(event_type, details);
    }
}
