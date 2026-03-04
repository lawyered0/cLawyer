//! Matter conflict-check handlers.

use std::sync::Arc;

use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::{
    MatterConflictCheckRequest, MatterConflictCheckResponse, MatterConflictGraphReindexResponse,
    MatterIntakeConflictCheckRequest, MatterIntakeConflictCheckResponse,
};
use crate::db::AuditSeverity;

const MAX_CONFLICT_TEXT_PREVIEW_CHARS: usize = 100;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route(
            "/api/matters/conflict-check",
            post(matters_conflict_check_handler),
        )
        .route(
            "/api/matters/conflicts/check",
            post(matters_conflicts_check_handler),
        )
        .route(
            "/api/matters/conflicts/reindex",
            post(matters_conflicts_reindex_handler),
        )
}

fn conflict_text_preview(text: &str) -> String {
    let normalized = text
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let collapsed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = collapsed
        .chars()
        .take(MAX_CONFLICT_TEXT_PREVIEW_CHARS)
        .collect();
    if collapsed.chars().count() > MAX_CONFLICT_TEXT_PREVIEW_CHARS {
        format!("{preview}...")
    } else {
        preview
    }
}

pub(crate) async fn matters_conflict_check_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MatterIntakeConflictCheckRequest>,
) -> Result<Json<MatterIntakeConflictCheckResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let legal = crate::channels::web::server::legal_config_for_gateway(state.as_ref());
    if !legal.enabled || !legal.conflict_check_enabled {
        return Err((
            StatusCode::CONFLICT,
            "Conflict check is disabled by legal policy".to_string(),
        ));
    }

    let matter_id = crate::legal::policy::sanitize_matter_id(req.matter_id.trim());
    if matter_id.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'matter_id' is empty after sanitization".to_string(),
        ));
    }

    let client_names = crate::channels::web::server::parse_matter_list(req.client_names);
    if client_names.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'client_names' must include at least one non-empty name".to_string(),
        ));
    }
    crate::channels::web::server::validate_intake_party_list("client_names", &client_names)?;
    let adversary_names = crate::channels::web::server::parse_matter_list(req.adversary_names);
    crate::channels::web::server::validate_intake_party_list("adversary_names", &adversary_names)?;

    let mut checked_parties: Vec<String> = Vec::new();
    for value in client_names.iter().chain(adversary_names.iter()) {
        if checked_parties
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(value))
        {
            continue;
        }
        checked_parties.push(value.clone());
    }
    if checked_parties.len() > crate::channels::web::server::MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "combined client/adversary names may include at most {} values",
                crate::channels::web::server::MAX_INTAKE_CONFLICT_PARTIES
            ),
        ));
    }

    let hits = store
        .find_conflict_hits_for_names(&checked_parties, 100)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let matched = !hits.is_empty();
    let top_conflict = hits.first().map(|hit| hit.party.clone());

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "matter_intake_conflict_check",
        state.user_id.as_str(),
        Some(matter_id.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "matter_id": matter_id.clone(),
            "matched": matched,
            "hit_count": hits.len(),
            "top_conflict": top_conflict,
            "checked_party_count": checked_parties.len(),
            "checked_by": state.user_id.clone(),
        }),
    )
    .await;
    if matched {
        crate::channels::web::server::record_legal_audit_event(
            state.as_ref(),
            "conflict_detected",
            state.user_id.as_str(),
            Some(matter_id.as_str()),
            AuditSeverity::Warn,
            serde_json::json!({
                "source": "intake_conflict_check",
                "hit_count": hits.len(),
                "top_conflict": hits.first().map(|hit| hit.party.clone()),
            }),
        )
        .await;
    }

    Ok(Json(MatterIntakeConflictCheckResponse {
        matched,
        hits,
        matter_id,
        checked_parties,
    }))
}

pub(crate) async fn matters_conflicts_check_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MatterConflictCheckRequest>,
) -> Result<Json<MatterConflictCheckResponse>, (StatusCode, String)> {
    let text = req.text.trim();
    if text.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'text' must not be empty".to_string(),
        ));
    }
    if text.len() > crate::channels::web::server::MAX_CONFLICT_CHECK_TEXT_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'text' must be at most {} bytes",
                crate::channels::web::server::MAX_CONFLICT_CHECK_TEXT_LEN
            ),
        ));
    }

    let mut legal = crate::channels::web::server::legal_config_for_gateway(state.as_ref());
    if !legal.enabled || !legal.conflict_check_enabled {
        return Err((
            StatusCode::CONFLICT,
            "Conflict check is disabled by legal policy".to_string(),
        ));
    }

    let effective_matter_id = if let Some(override_id) = req.matter_id {
        let trimmed = override_id.trim();
        if trimmed.is_empty() {
            None
        } else {
            let sanitized = crate::legal::policy::sanitize_matter_id(trimmed);
            if sanitized.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "'matter_id' is empty after sanitization".to_string(),
                ));
            }
            Some(sanitized)
        }
    } else {
        crate::channels::web::server::load_active_matter_for_chat(state.as_ref()).await
    };
    if legal.require_matter_context && effective_matter_id.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Active matter is required by legal policy for conflict checks".to_string(),
        ));
    }

    legal.active_matter = effective_matter_id.clone();
    let db_available = state.store.is_some();
    let db_hits = if let Some(store) = state.store.as_ref() {
        match store
            .find_conflict_hits_for_text(text, legal.active_matter.as_deref(), 50)
            .await
        {
            Ok(hits) => hits,
            Err(err) => {
                tracing::warn!(
                    "DB text conflict check failed, falling back to workspace cache: {err}"
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let conflict = if let Some(first_hit) = db_hits.first() {
        Some(first_hit.party.clone())
    } else if db_available && !legal.conflict_file_fallback_enabled {
        None
    } else {
        let workspace = state.workspace.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Workspace not available".to_string(),
        ))?;
        crate::legal::matter::detect_conflict_with_store(None, workspace.as_ref(), &legal, text)
            .await
    };
    let matched = conflict.is_some();

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "matter_conflict_check",
        state.user_id.as_str(),
        effective_matter_id.as_deref(),
        AuditSeverity::Info,
        serde_json::json!({
            "matter_id": effective_matter_id.clone(),
            "matched": matched,
            "conflict": conflict.clone(),
            "text_preview": conflict_text_preview(text),
            "checked_by": state.user_id.clone(),
            "source": "manual_text_check",
        }),
    )
    .await;
    if matched {
        crate::channels::web::server::record_legal_audit_event(
            state.as_ref(),
            "conflict_detected",
            state.user_id.as_str(),
            effective_matter_id.as_deref(),
            AuditSeverity::Warn,
            serde_json::json!({
                "source": "manual_text_check",
                "conflict": conflict.clone(),
                "hit_count": db_hits.len(),
            }),
        )
        .await;
    }

    Ok(Json(MatterConflictCheckResponse {
        matched,
        conflict,
        matter_id: effective_matter_id,
        hits: db_hits,
    }))
}

pub(crate) async fn matters_conflicts_reindex_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MatterConflictGraphReindexResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let legal = crate::channels::web::server::legal_config_for_gateway(state.as_ref());
    if !legal.enabled || !legal.conflict_check_enabled {
        return Err((
            StatusCode::CONFLICT,
            "Conflict reindex is disabled by legal policy".to_string(),
        ));
    }

    let report = crate::legal::matter::reindex_conflict_graph(workspace.as_ref(), store, &legal)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "conflict_graph_reindexed",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "triggered_by": state.user_id.clone(),
            "scanned_matters": report.scanned_matters,
            "seeded_matters": report.seeded_matters,
            "skipped_matters": report.skipped_matters,
            "global_conflicts_seeded": report.global_conflicts_seeded,
            "global_aliases_seeded": report.global_aliases_seeded,
            "warning_count": report.warnings.len(),
        }),
    )
    .await;

    Ok(Json(MatterConflictGraphReindexResponse {
        status: "ok",
        report,
    }))
}
