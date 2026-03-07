//! Matter conflict-check handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::channels::web::auth::RequestPrincipal;
use crate::channels::web::handlers::helpers::matter::require_matter_access;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::{
    CreatePartyRelationshipRequest, MatterConflictCheckRequest, MatterConflictCheckResponse,
    MatterConflictClearanceRequest, MatterConflictClearanceResponse,
    MatterConflictGraphReindexResponse, MatterConflictReportResponse,
    MatterIntakeConflictCheckRequest, MatterIntakeConflictCheckResponse, MatterPartiesResponse,
    MatterPartyRelationshipResponse, UpsertMatterPartyRequest,
};
use crate::db::{AuditSeverity, MatterMemberRole, PartyRole, UpsertMatterPartyParams};

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
        .route(
            "/api/matters/{id}/parties",
            get(matter_parties_list_handler).post(matter_parties_upsert_handler),
        )
        .route(
            "/api/matters/{id}/parties/relationships",
            post(matter_parties_relationships_handler),
        )
        .route(
            "/api/matters/{id}/conflicts/report",
            get(matter_conflicts_report_handler),
        )
        .route(
            "/api/matters/{id}/conflicts/clearance",
            post(matter_conflicts_clearance_handler),
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

    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
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

    let mut legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
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

    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;
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

fn parse_party_role(raw: &str) -> Result<PartyRole, (StatusCode, String)> {
    PartyRole::from_db_value(&raw.trim().to_ascii_lowercase()).ok_or((
        StatusCode::BAD_REQUEST,
        "role must be one of client, adverse, related, witness, affiliate, principal, or opposing_counsel".to_string(),
    ))
}

async fn ensure_structured_matter_parties(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    if !store
        .list_matter_parties(matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .is_empty()
    {
        return Ok(());
    }
    let Some(workspace) = state.workspace.as_ref() else {
        return Ok(());
    };
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state);
    let metadata = match crate::legal::matter::read_matter_metadata_for_root(
        workspace.as_ref(),
        &matter_root,
        matter_id,
    )
    .await
    {
        Ok(metadata) => metadata,
        Err(_) => return Ok(()),
    };
    store
        .upsert_matter_party(
            matter_id,
            &UpsertMatterPartyParams {
                name: metadata.client,
                role: PartyRole::Client,
                aliases: Vec::new(),
                notes: None,
                opened_at: None,
                closed_at: None,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    for adversary in metadata.adversaries {
        store
            .upsert_matter_party(
                matter_id,
                &UpsertMatterPartyParams {
                    name: adversary,
                    role: PartyRole::Adverse,
                    aliases: Vec::new(),
                    notes: None,
                    opened_at: None,
                    closed_at: None,
                },
            )
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }
    Ok(())
}

async fn build_matter_conflict_report(
    state: &GatewayState,
    matter_id: &str,
) -> Result<MatterConflictReportResponse, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    ensure_structured_matter_parties(state, matter_id).await?;
    let parties = store
        .list_matter_parties(matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let relationships = store
        .list_matter_party_relationships(matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let checked_parties = parties
        .iter()
        .map(|party| party.name.clone())
        .collect::<Vec<_>>();
    let mut hits = store
        .find_conflict_hits_for_names(&checked_parties, 100)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    hits.retain(|hit| hit.matter_id != matter_id);
    let latest_clearance = store
        .latest_conflict_clearance(matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .map(crate::channels::web::server::conflict_clearance_info_to_response);
    Ok(MatterConflictReportResponse {
        matter_id: matter_id.to_string(),
        checked_parties,
        parties: parties
            .into_iter()
            .map(crate::channels::web::server::matter_party_record_to_info)
            .collect(),
        relationships: relationships
            .into_iter()
            .map(crate::channels::web::server::party_relationship_record_to_info)
            .collect(),
        hits,
        latest_clearance,
    })
}

pub(crate) async fn matter_parties_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterPartiesResponse>, (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    ensure_structured_matter_parties(state.as_ref(), &matter_id).await?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let parties = store
        .list_matter_parties(&matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(MatterPartiesResponse {
        matter_id,
        parties: parties
            .into_iter()
            .map(crate::channels::web::server::matter_party_record_to_info)
            .collect(),
    }))
}

pub(crate) async fn matter_parties_upsert_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<UpsertMatterPartyRequest>,
) -> Result<
    (
        StatusCode,
        Json<crate::channels::web::types::MatterPartyInfo>,
    ),
    (StatusCode, String),
> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let name = crate::channels::web::server::parse_required_matter_field("name", &req.name)?;
    let role = parse_party_role(&req.role)?;
    let aliases = req
        .aliases
        .into_iter()
        .filter_map(|alias| crate::channels::web::server::parse_optional_matter_field(Some(alias)))
        .collect::<Vec<_>>();
    let notes = crate::channels::web::server::parse_optional_matter_field(req.notes);
    let opened_at =
        crate::channels::web::server::parse_optional_datetime("opened_at", req.opened_at)?;
    let closed_at =
        crate::channels::web::server::parse_optional_datetime("closed_at", req.closed_at)?;
    let party = store
        .upsert_matter_party(
            &matter_id,
            &UpsertMatterPartyParams {
                name,
                role,
                aliases,
                notes,
                opened_at,
                closed_at,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::matter_party_record_to_info(
            party,
        )),
    ))
}

pub(crate) async fn matter_parties_relationships_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<CreatePartyRelationshipRequest>,
) -> Result<Json<MatterPartyRelationshipResponse>, (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let relationship = store
        .upsert_party_relationship(&crate::db::CreatePartyRelationshipParams {
            parent_party_id: req
                .parent_party_id
                .as_deref()
                .map(|value| crate::channels::web::server::parse_uuid(value, "parent_party_id"))
                .transpose()?,
            parent_name: crate::channels::web::server::parse_optional_matter_field(req.parent_name),
            child_party_id: req
                .child_party_id
                .as_deref()
                .map(|value| crate::channels::web::server::parse_uuid(value, "child_party_id"))
                .transpose()?,
            child_name: crate::channels::web::server::parse_optional_matter_field(req.child_name),
            kind: crate::channels::web::server::parse_required_matter_field("kind", &req.kind)?,
        })
        .await
        .map_err(|err| (StatusCode::BAD_REQUEST, err.to_string()))?;
    Ok(Json(MatterPartyRelationshipResponse {
        relationship: crate::channels::web::server::party_relationship_record_to_info(relationship),
    }))
}

pub(crate) async fn matter_conflicts_report_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterConflictReportResponse>, (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    Ok(Json(
        build_matter_conflict_report(state.as_ref(), &matter_id).await?,
    ))
}

pub(crate) async fn matter_conflicts_clearance_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<MatterConflictClearanceRequest>,
) -> Result<Json<MatterConflictClearanceResponse>, (StatusCode, String)> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let report = build_matter_conflict_report(state.as_ref(), &matter_id).await?;
    crate::channels::web::handlers::matters::core::persist_conflict_clearance_decision(
        &state,
        &matter_id,
        req.decision,
        crate::channels::web::server::parse_optional_matter_field(req.note),
        req.reviewing_attorney,
        &report.hits,
        "manual_conflict_report",
    )
    .await?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let latest = store
        .latest_conflict_clearance(&matter_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Conflict clearance was not recorded".to_string(),
        ))?;
    Ok(Json(MatterConflictClearanceResponse {
        matter_id,
        decision: req.decision.as_str().to_string(),
        hit_count: report.hits.len(),
        latest_clearance: crate::channels::web::server::conflict_clearance_info_to_response(latest),
    }))
}
