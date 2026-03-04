//! Legal and compliance handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use chrono::Utc;

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{AuditEventQuery as DbAuditEventQuery, AuditSeverity};
use crate::llm::{ChatMessage, CompletionRequest};

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/legal/audit", get(legal_audit_list_handler))
        .route("/api/legal/court-rules", get(legal_court_rules_handler))
        .route("/api/compliance/status", get(compliance_status_handler))
        .route("/api/compliance/letter", post(compliance_letter_handler))
}

fn court_rule_to_info(rule: &crate::legal::calendar::CourtRule) -> CourtRuleInfo {
    CourtRuleInfo {
        id: rule.id.clone(),
        citation: rule.citation.clone(),
        deadline_type: rule.deadline_type.as_str().to_string(),
        offset_days: rule.offset_days,
        court_days: rule.court_days,
    }
}

pub(crate) async fn legal_court_rules_handler()
-> Result<Json<CourtRulesResponse>, (StatusCode, String)> {
    let rules = crate::legal::calendar::all_court_rules()
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err))?;
    let payload = rules.iter().map(court_rule_to_info).collect::<Vec<_>>();
    Ok(Json(CourtRulesResponse { rules: payload }))
}

pub(crate) async fn legal_audit_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<crate::channels::web::server::LegalAuditQuery>,
) -> Result<Json<LegalAuditListResponse>, (StatusCode, String)> {
    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
    if !legal.audit.enabled {
        return Err((
            StatusCode::NOT_FOUND,
            "Legal audit logging is disabled".to_string(),
        ));
    }
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let limit = query.limit.unwrap_or(50);
    if limit == 0 || limit > 200 {
        return Err((
            StatusCode::BAD_REQUEST,
            "'limit' must be between 1 and 200".to_string(),
        ));
    }
    let offset = query.offset.unwrap_or(0);
    let event_type_filter = query.event_type.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let matter_id_filter = query.matter_id.as_ref().and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });
    let severity_filter =
        crate::channels::web::server::parse_audit_severity_query(query.severity.as_deref())?;
    // Keep backward compatibility with existing `from`/`to` query names.
    let since_ts = crate::channels::web::server::parse_utc_query_ts(
        "since",
        query.since.as_deref().or(query.from.as_deref()),
    )?;
    let until_ts = crate::channels::web::server::parse_utc_query_ts(
        "until",
        query.until.as_deref().or(query.to.as_deref()),
    )?;

    if let (Some(since), Some(until)) = (since_ts, until_ts)
        && since > until
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "'since' must be earlier than or equal to 'until'".to_string(),
        ));
    }

    let db_query = DbAuditEventQuery {
        event_type: event_type_filter,
        matter_id: matter_id_filter,
        severity: severity_filter,
        since: since_ts,
        until: until_ts,
    };
    let total = store
        .count_audit_events(&state.user_id, &db_query)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let events = store
        .list_audit_events(&state.user_id, &db_query, limit, offset)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::audit_event_record_to_info)
        .collect::<Vec<_>>();
    let next_offset = if offset + events.len() < total {
        Some(offset + events.len())
    } else {
        None
    };

    Ok(Json(LegalAuditListResponse {
        events,
        total,
        next_offset,
    }))
}

fn compliance_status_level(level: crate::compliance::ComplianceState) -> ComplianceStatusLevel {
    match level {
        crate::compliance::ComplianceState::Compliant => ComplianceStatusLevel::Compliant,
        crate::compliance::ComplianceState::Partial => ComplianceStatusLevel::Partial,
        crate::compliance::ComplianceState::NeedsReview => ComplianceStatusLevel::NeedsReview,
    }
}

fn compliance_function_to_response(
    function: &crate::compliance::ComplianceFunction,
) -> ComplianceFunctionStatus {
    ComplianceFunctionStatus {
        status: compliance_status_level(function.status),
        checks: function
            .checks
            .iter()
            .map(|check| ComplianceCheckResult {
                id: check.id.to_string(),
                label: check.label.to_string(),
                status: compliance_status_level(check.status),
                detail: check.detail.clone(),
            })
            .collect(),
    }
}

fn normalize_compliance_framework(raw: Option<&str>) -> Result<String, (StatusCode, String)> {
    let framework = raw
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("nist")
        .to_ascii_lowercase();
    match framework.as_str() {
        "nist" | "colorado-sb205" | "eu-ai-act" => Ok(framework),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'framework' must be one of: nist, colorado-sb205, eu-ai-act".to_string(),
        )),
    }
}

fn is_matter_classified(metadata: &crate::legal::matter::MatterMetadata) -> bool {
    !metadata.confidentiality.trim().is_empty() && !metadata.retention.trim().is_empty()
}

async fn collect_compliance_matter_metrics(state: &GatewayState) -> (usize, usize, Vec<String>) {
    let mut data_gaps = Vec::new();
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state);

    if let Some(store) = state.store.as_ref() {
        match store.list_matters_db(&state.user_id).await {
            Ok(rows) => {
                let total = rows.len();
                let mut classified = 0usize;
                if let Some(workspace) = state.workspace.as_ref() {
                    for row in rows {
                        if let Some(metadata) =
                            crate::channels::web::server::read_workspace_matter_metadata_optional(
                                Some(workspace),
                                &matter_root,
                                &row.matter_id,
                            )
                            .await
                            && is_matter_classified(&metadata)
                        {
                            classified += 1;
                        }
                    }
                } else {
                    data_gaps.push(
                        "Workspace unavailable; matter classification coverage may be understated."
                            .to_string(),
                    );
                }
                return (total, classified, data_gaps);
            }
            Err(err) => {
                data_gaps.push(format!(
                    "Failed to list matters from database; falling back to workspace scan: {err}"
                ));
            }
        }
    }

    let Some(workspace) = state.workspace.as_ref() else {
        data_gaps.push(
            "Matter metrics unavailable: both database and workspace are unavailable.".to_string(),
        );
        return (0, 0, data_gaps);
    };

    let entries = match crate::channels::web::server::list_matters_root_entries(
        workspace.list(&matter_root).await,
    ) {
        Ok(entries) => entries,
        Err((_status, message)) => {
            data_gaps.push(format!(
                "Failed to list matter root '{}': {}",
                matter_root, message
            ));
            return (0, 0, data_gaps);
        }
    };

    let mut total = 0usize;
    let mut classified = 0usize;
    for entry in entries.into_iter().filter(|entry| entry.is_directory) {
        let matter_id = entry.path.rsplit('/').next().unwrap_or("");
        if matter_id.is_empty() || matter_id == "_template" {
            continue;
        }
        total += 1;
        if let Some(metadata) =
            crate::channels::web::server::read_workspace_matter_metadata_optional(
                Some(workspace),
                &matter_root,
                matter_id,
            )
            .await
            && is_matter_classified(&metadata)
        {
            classified += 1;
        }
    }

    (total, classified, data_gaps)
}

async fn collect_compliance_tool_count(state: &GatewayState) -> (usize, Vec<String>) {
    let mut data_gaps = Vec::new();
    let count = if let Some(registry) = state.tool_registry.as_ref() {
        registry.tool_definitions().await.len()
    } else {
        data_gaps.push("Tool registry unavailable; inventory count reported as 0.".to_string());
        0
    };
    (count, data_gaps)
}

async fn collect_compliance_audit_metrics(
    state: &GatewayState,
) -> (
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Vec<String>,
) {
    let mut data_gaps = Vec::new();
    let Some(store) = state.store.as_ref() else {
        data_gaps.push("Audit metrics unavailable because database is not configured.".to_string());
        return (None, None, None, None, None, data_gaps);
    };

    let base_query = DbAuditEventQuery::default();
    let total = match store.count_audit_events(&state.user_id, &base_query).await {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!("Failed to read audit event count: {err}"));
            None
        }
    };

    let severity_count = |severity: AuditSeverity| DbAuditEventQuery {
        severity: Some(severity),
        ..DbAuditEventQuery::default()
    };

    let info_count = match store
        .count_audit_events(&state.user_id, &severity_count(AuditSeverity::Info))
        .await
    {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!("Failed to read info audit count: {err}"));
            None
        }
    };

    let warn_count = match store
        .count_audit_events(&state.user_id, &severity_count(AuditSeverity::Warn))
        .await
    {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!("Failed to read warn audit count: {err}"));
            None
        }
    };

    let critical_count = match store
        .count_audit_events(&state.user_id, &severity_count(AuditSeverity::Critical))
        .await
    {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!("Failed to read critical audit count: {err}"));
            None
        }
    };

    let approval_required_count = match store
        .count_audit_events(
            &state.user_id,
            &DbAuditEventQuery {
                event_type: Some("approval_required".to_string()),
                ..DbAuditEventQuery::default()
            },
        )
        .await
    {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!(
                "Failed to read approval-required audit count: {err}"
            ));
            None
        }
    };

    let approval_decision_count = match store
        .count_audit_events(
            &state.user_id,
            &DbAuditEventQuery {
                event_type: Some("approval_decision".to_string()),
                ..DbAuditEventQuery::default()
            },
        )
        .await
    {
        Ok(value) => Some(value),
        Err(err) => {
            data_gaps.push(format!(
                "Failed to read approval-decision audit count: {err}"
            ));
            None
        }
    };

    let approval_events_total = match (approval_required_count, approval_decision_count) {
        (Some(required), Some(decision)) => Some(required + decision),
        (Some(required), None) => Some(required),
        (None, Some(decision)) => Some(decision),
        (None, None) => None,
    };

    (
        total,
        approval_events_total,
        info_count,
        warn_count,
        critical_count,
        data_gaps,
    )
}

async fn build_compliance_status(
    state: &GatewayState,
    legal: &crate::config::LegalConfig,
) -> crate::compliance::ComplianceStatus {
    let (matters_total, matters_classified, matter_gaps) =
        collect_compliance_matter_metrics(state).await;
    let (tools_total, tool_gaps) = collect_compliance_tool_count(state).await;
    let (
        audit_events_total,
        approval_events_total,
        audit_info_count,
        audit_warn_count,
        audit_critical_count,
        audit_gaps,
    ) = collect_compliance_audit_metrics(state).await;

    let mut data_gaps = Vec::new();
    data_gaps.extend(matter_gaps);
    data_gaps.extend(tool_gaps);
    data_gaps.extend(audit_gaps);

    let inputs = crate::compliance::ComplianceInputs {
        audit_enabled: legal.audit.enabled,
        audit_hash_chain: legal.audit.hash_chain,
        hardening_max_lockdown: legal.hardening
            == crate::config::LegalHardeningProfile::MaxLockdown,
        conflict_check_enabled: legal.conflict_check_enabled,
        conflict_file_fallback_enabled: legal.conflict_file_fallback_enabled,
        privilege_guard_enabled: legal.privilege_guard,
        network_deny_by_default: legal.network.deny_by_default,
        redaction_pii: legal.redaction.pii,
        redaction_phi: legal.redaction.phi,
        redaction_financial: legal.redaction.financial,
        matters_total,
        matters_classified,
        tools_total,
        audit_events_total,
        approval_events_total,
        audit_info_count,
        audit_warn_count,
        audit_critical_count,
        runtime: state.runtime_facts.clone(),
        data_gaps,
    };
    crate::compliance::evaluate_nist_rmf(&inputs)
}

pub(crate) async fn compliance_status_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ComplianceStatusResponse>, (StatusCode, String)> {
    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
    let status = build_compliance_status(state.as_ref(), &legal).await;
    let generated_at = Utc::now().to_rfc3339();

    let response = ComplianceStatusResponse {
        overall: compliance_status_level(status.overall),
        govern: compliance_function_to_response(&status.govern),
        map: compliance_function_to_response(&status.map),
        measure: compliance_function_to_response(&status.measure),
        manage: compliance_function_to_response(&status.manage),
        metrics: ComplianceMetrics {
            matters_total: status.metrics.matters_total,
            matters_classified: status.metrics.matters_classified,
            tools_total: status.metrics.tools_total,
            audit_events_total: status.metrics.audit_events_total,
            audit_info_count: status.metrics.audit_info_count,
            audit_warn_count: status.metrics.audit_warn_count,
            audit_critical_count: status.metrics.audit_critical_count,
            safety_policy_rule_count: state.runtime_facts.safety_policy_rule_count,
            safety_leak_pattern_count: state.runtime_facts.safety_leak_pattern_count,
        },
        data_gaps: status.data_gaps.clone(),
        generated_at: generated_at.clone(),
    };

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "compliance_status_viewed",
        "gateway",
        legal.active_matter.as_deref(),
        AuditSeverity::Info,
        serde_json::json!({
            "overall": response.overall,
            "generated_at": generated_at,
            "data_gap_count": response.data_gaps.len(),
        }),
    )
    .await;

    Ok(Json(response))
}

pub(crate) async fn compliance_letter_handler(
    State(state): State<Arc<GatewayState>>,
    body: Option<Json<ComplianceLetterRequest>>,
) -> Result<Json<ComplianceLetterResponse>, (StatusCode, String)> {
    let req = body.map(|Json(value)| value).unwrap_or_default();
    let framework = normalize_compliance_framework(req.framework.as_deref())?;
    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
    let status = build_compliance_status(state.as_ref(), &legal).await;
    let llm = state.llm_provider.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "LLM provider is not available".to_string(),
    ))?;
    let model = llm.active_model_name();
    let generated_at = Utc::now().to_rfc3339();
    let prompt = crate::compliance::build_attestation_prompt(
        &framework,
        req.firm_name.as_deref(),
        &generated_at,
        &legal,
        &status,
        &model,
    );

    let request = CompletionRequest::new(vec![
        ChatMessage::system(
            "You write factual operational attestation letters from provided runtime evidence only.",
        ),
        ChatMessage::user(prompt),
    ])
    .with_temperature(0.1)
    .with_max_tokens(1800);

    let completion = llm.complete(request).await.map_err(|err| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Failed to generate compliance letter: {}", err),
        )
    })?;
    let base_markdown = completion.content.trim();
    if base_markdown.is_empty() {
        return Err((
            StatusCode::BAD_GATEWAY,
            "LLM returned an empty compliance letter".to_string(),
        ));
    }

    let markdown = format!(
        "{base_markdown}\n\n---\n\n*Disclaimer: Configuration summary only; not legal advice.*\n"
    );
    let overall = compliance_status_level(status.overall);

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "compliance_letter_generated",
        "gateway",
        legal.active_matter.as_deref(),
        AuditSeverity::Info,
        serde_json::json!({
            "framework": framework,
            "model": model,
            "overall": overall,
            "firm_name": req.firm_name,
            "generated_at": generated_at,
        }),
    )
    .await;

    Ok(Json(ComplianceLetterResponse {
        framework,
        model,
        generated_at,
        overall,
        markdown,
    }))
}
