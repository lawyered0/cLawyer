//! Matter and client core handlers.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::channels::web::auth::RequestPrincipal;
use crate::channels::web::handlers::helpers::matter::require_matter_access;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    AuditSeverity, ClientType, ConflictClearanceRecord, ConflictDecision, ConflictHit,
    CreateClientParams, CreateMatterDeadlineParams, MatterMemberRole, MatterStatus,
    OverrideDeadlineParams, UpdateClientParams, UpdateMatterDeadlineParams, UpdateMatterParams,
    UpsertMatterMembershipParams, UpsertMatterParams,
};

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ClientsQuery {
    pub(crate) q: Option<String>,
}

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route(
            "/api/matters",
            get(matters_list_handler).post(matters_create_handler),
        )
        .route(
            "/api/clients",
            get(clients_list_handler).post(clients_create_handler),
        )
        .route(
            "/api/clients/{id}",
            get(clients_get_handler)
                .patch(clients_patch_handler)
                .delete(clients_delete_handler),
        )
        .route(
            "/api/matters/{id}",
            get(matter_get_handler)
                .patch(matter_patch_handler)
                .delete(matter_delete_handler),
        )
        .route(
            "/api/matters/active",
            get(matters_active_get_handler).post(matters_active_set_handler),
        )
        .route(
            "/api/matters/{id}/members",
            get(matter_members_list_handler),
        )
        .route(
            "/api/matters/{id}/members/{member_user_id}",
            axum::routing::put(matter_member_upsert_handler).delete(matter_member_remove_handler),
        )
        .route(
            "/api/matters/{id}/deadlines",
            get(matter_deadlines_handler).post(matter_deadlines_create_handler),
        )
        .route(
            "/api/matters/{id}/deadlines/{deadline_id}",
            axum::routing::patch(matter_deadlines_patch_handler)
                .delete(matter_deadlines_delete_handler),
        )
        .route(
            "/api/matters/{id}/deadlines/compute",
            post(matter_deadlines_compute_handler),
        )
        .route(
            "/api/matters/{id}/deadlines/{deadline_id}/override",
            post(matter_deadline_override_handler),
        )
        .route(
            "/api/matters/{id}/deadlines/{deadline_id}/audit",
            get(matter_deadline_audit_handler),
        )
}

const MAX_INTAKE_CONFLICT_PARTY_CHARS: usize = 160;

fn validate_intake_party_name(field_name: &str, value: &str) -> Result<(), (StatusCode, String)> {
    if value.chars().count() > MAX_INTAKE_CONFLICT_PARTY_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' must be at most {} characters",
                field_name, MAX_INTAKE_CONFLICT_PARTY_CHARS
            ),
        ));
    }
    Ok(())
}

fn build_checked_parties(client: &str, adversaries: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if !client.trim().is_empty() {
        out.push(client.trim().to_string());
    }
    for name in adversaries {
        let trimmed = name.trim();
        if trimmed.is_empty()
            || out
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(trimmed))
        {
            continue;
        }
        out.push(trimmed.to_string());
    }
    out
}

fn json_error_string(value: serde_json::Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| "{\"error\":\"serialization_error\"}".into())
}

fn conflict_required_error(hits: &[ConflictHit]) -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        json_error_string(serde_json::json!({
            "error": "Potential conflicts detected. Review and submit a conflict decision before creating the matter.",
            "conflict_required": true,
            "hits": hits,
        })),
    )
}

fn conflict_declined_error(hits: &[ConflictHit]) -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        json_error_string(serde_json::json!({
            "error": "Matter creation declined due to conflict review decision.",
            "decision": "declined",
            "hits": hits,
        })),
    )
}

fn active_set_conflict_required_error(hits: &[ConflictHit]) -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        json_error_string(serde_json::json!({
            "error": "Potential conflicts detected for this matter. Review and submit a conflict decision before setting it active.",
            "conflict_required": true,
            "hits": hits,
        })),
    )
}

fn active_set_conflict_declined_error(hits: &[ConflictHit]) -> (StatusCode, String) {
    (
        StatusCode::CONFLICT,
        json_error_string(serde_json::json!({
            "error": "Active-matter selection declined due to conflict review decision.",
            "decision": "declined",
            "hits": hits,
        })),
    )
}

fn decision_requires_note(decision: ConflictDecision) -> bool {
    matches!(
        decision,
        ConflictDecision::Waived | ConflictDecision::Declined
    )
}

fn decision_allows_matter_activation(decision: ConflictDecision) -> bool {
    matches!(decision, ConflictDecision::Clear | ConflictDecision::Waived)
}

fn conflict_hit_signature(hit: &ConflictHit) -> String {
    format!(
        "{}|{}|{}|{}",
        hit.matter_id.to_ascii_lowercase(),
        hit.party.to_ascii_lowercase(),
        hit.role.as_str(),
        hit.matched_via.to_ascii_lowercase()
    )
}

fn conflict_signature_for_hits(hits: &[ConflictHit]) -> Vec<String> {
    let mut values: Vec<String> = hits.iter().map(conflict_hit_signature).collect();
    values.sort();
    values.dedup();
    values
}

fn clearance_signature_matches_hits(
    matter_id: &str,
    hits: &[ConflictHit],
    hits_json: &serde_json::Value,
) -> bool {
    let Ok(stored_hits) = serde_json::from_value::<Vec<ConflictHit>>(hits_json.clone()) else {
        tracing::error!(
            "failed to parse stored conflict clearance hits_json for matter '{}'",
            matter_id
        );
        return false;
    };
    conflict_signature_for_hits(&stored_hits) == conflict_signature_for_hits(hits)
}

pub(crate) async fn persist_conflict_clearance_decision(
    state: &Arc<GatewayState>,
    matter_id: &str,
    decision: ConflictDecision,
    note: Option<String>,
    reviewing_attorney: Option<String>,
    hits: &[ConflictHit],
    source: &str,
) -> Result<(), (StatusCode, String)> {
    if decision_requires_note(decision) && note.as_deref().is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'conflict_note' is required for waived or declined decisions".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let serialized_hits = serde_json::to_vec(hits).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&serialized_hits);
    let report_hash = format!("sha256:{:x}", hasher.finalize());
    let clearance = ConflictClearanceRecord {
        matter_id: matter_id.to_string(),
        checked_by: state.user_id.clone(),
        cleared_by: if matches!(decision, ConflictDecision::Declined) {
            None
        } else {
            Some(state.user_id.clone())
        },
        decision,
        note: note.clone(),
        hits_json: serde_json::to_value(hits).unwrap_or_else(|err| {
            tracing::error!(
                "failed to serialize conflict hits for matter '{}': {}",
                matter_id,
                err
            );
            serde_json::json!([])
        }),
        hit_count: hits.len() as i32,
        reviewing_attorney: reviewing_attorney.or_else(|| Some(state.user_id.clone())),
        report_hash: Some(report_hash.clone()),
        signed_at: if decision_allows_matter_activation(decision) {
            Some(chrono::Utc::now())
        } else {
            None
        },
    };
    store
        .record_conflict_clearance(&clearance)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "conflict_clearance_decision",
        state.user_id.as_str(),
        Some(matter_id),
        AuditSeverity::Info,
        serde_json::json!({
            "matter_id": matter_id,
            "decision": decision.as_str(),
            "checked_by": state.user_id.clone(),
            "cleared_by_present": clearance.cleared_by.is_some(),
            "hit_count": clearance.hit_count,
            "report_hash": report_hash,
            "source": source,
        }),
    )
    .await;

    Ok(())
}

pub(crate) async fn matters_create_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Json(req): Json<CreateMatterRequest>,
) -> Result<(StatusCode, Json<CreateMatterResponse>), (StatusCode, String)> {
    // Only the matter owner can create matters.
    if state.store.is_some() && principal.user_id != state.user_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());

    let raw_matter_id =
        crate::channels::web::server::parse_required_matter_field("matter_id", &req.matter_id)?;
    let sanitized = crate::legal::policy::sanitize_matter_id(&raw_matter_id);
    if sanitized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Matter ID is empty after sanitization".to_string(),
        ));
    }

    let existing = crate::channels::web::server::list_matters_root_entries(
        workspace.list(&matter_root).await,
    )?;
    let matter_prefix = format!("{matter_root}/{sanitized}");
    if existing
        .iter()
        .any(|entry| entry.is_directory && entry.path == matter_prefix)
    {
        return Err((
            StatusCode::CONFLICT,
            format!("Matter '{}' already exists", sanitized),
        ));
    }
    if store
        .get_matter_db(&state.user_id, &sanitized)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            format!("Matter '{}' already exists", sanitized),
        ));
    }

    let client = crate::channels::web::server::parse_required_matter_field("client", &req.client)?;
    let confidentiality = crate::channels::web::server::parse_required_matter_field(
        "confidentiality",
        &req.confidentiality,
    )?;
    let retention =
        crate::channels::web::server::parse_required_matter_field("retention", &req.retention)?;
    validate_intake_party_name("client", &client)?;
    let jurisdiction = crate::channels::web::server::parse_optional_matter_field(req.jurisdiction);
    let practice_area =
        crate::channels::web::server::parse_optional_matter_field(req.practice_area);
    let opened_date = crate::channels::web::server::parse_optional_matter_field(
        req.opened_date.or(req.opened_at),
    );
    crate::channels::web::server::validate_optional_matter_field_length(
        "jurisdiction",
        &jurisdiction,
    )?;
    crate::channels::web::server::validate_optional_matter_field_length(
        "practice_area",
        &practice_area,
    )?;
    if let Some(value) = opened_date.as_deref() {
        crate::channels::web::server::validate_opened_date(value)?;
    }
    let team = crate::channels::web::server::parse_matter_list(req.team);
    let adversaries = crate::channels::web::server::parse_matter_list(req.adversaries);
    crate::channels::web::server::validate_intake_party_list("adversaries", &adversaries)?;
    let conflict_decision = req.conflict_decision;
    let conflict_note =
        crate::channels::web::server::parse_optional_matter_field(req.conflict_note);
    let checked_parties = build_checked_parties(&client, &adversaries);
    if checked_parties.len() > crate::channels::web::server::MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "combined conflict-check parties may include at most {} names",
                crate::channels::web::server::MAX_INTAKE_CONFLICT_PARTIES
            ),
        ));
    }
    let conflict_hits = if checked_parties.is_empty() {
        Vec::new()
    } else {
        store
            .find_conflict_hits_for_names(&checked_parties, 50)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    if !conflict_hits.is_empty() {
        let decision = match conflict_decision {
            Some(decision) => decision,
            None => return Err(conflict_required_error(&conflict_hits)),
        };
        persist_conflict_clearance_decision(
            &state,
            &sanitized,
            decision,
            conflict_note.clone(),
            None,
            &conflict_hits,
            "create_flow",
        )
        .await?;

        if matches!(decision, ConflictDecision::Declined) {
            return Err(conflict_declined_error(&conflict_hits));
        }
    }

    let metadata = crate::legal::matter::MatterMetadata {
        matter_id: sanitized.clone(),
        client: client.clone(),
        team: team.clone(),
        confidentiality: confidentiality.clone(),
        adversaries: adversaries.clone(),
        retention: retention.clone(),
        jurisdiction: jurisdiction.clone(),
        practice_area: practice_area.clone(),
        opened_date: opened_date.clone(),
    };
    let matter_yaml = serde_yml::to_string(&metadata)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let scaffold = vec![
        (
            format!("{matter_prefix}/matter.yaml"),
            format!(
                "# Matter metadata schema\n# Required: matter_id, client, confidentiality, retention\n{}",
                matter_yaml
            ),
        ),
        (
            format!("{matter_prefix}/README.md"),
            format!(
                "# Matter {}\n\nClient: {}\n\nThis workspace stores privileged legal work product.\n\n## Suggested Workflow\n\n1. Intake and conflicts\n2. Facts and chronology\n3. Research and authority synthesis\n4. Drafting and review\n5. Filing and follow-up\n",
                sanitized, client
            ),
        ),
        (
            format!("{matter_prefix}/workflows/intake_checklist.md"),
            "# Intake Checklist\n\n- [ ] Confirm engagement and scope\n- [ ] Confirm client contact and billing details\n- [ ] Run conflict check and document result\n- [ ] Capture key deadlines and court dates\n- [ ] Identify required initial filings or responses\n".to_string(),
        ),
        (
            format!("{matter_prefix}/workflows/review_and_filing_checklist.md"),
            "# Review and Filing Checklist\n\n- [ ] Separate facts from analysis in final draft\n- [ ] Verify citation format coverage for factual/legal assertions\n- [ ] Confirm privilege/confidentiality review complete\n- [ ] Final QA pass and attorney approval recorded\n- [ ] Filing/service steps completed and logged\n".to_string(),
        ),
        (
            format!("{matter_prefix}/deadlines/calendar.md"),
            "# Deadlines and Hearings\n\n| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/facts/key_facts.md"),
            "# Key Facts Log\n\n| Fact | Source | Confidence | Notes |\n|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/research/authority_table.md"),
            "# Authority Table\n\n| Authority | Holding / Principle | Relevance | Risk / Limit | Citation |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/discovery/request_tracker.md"),
            "# Discovery Request Tracker\n\n| Request / Topic | Served / Received | Response Due | Status | Notes |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/communications/contact_log.md"),
            "# Communications Log\n\n| Date | With | Channel | Summary | Follow-up |\n|---|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/research_memo.md"),
            "# Research Memo Template\n\n## Question Presented\n\n## Brief Answer\n\n## Facts (Cited)\n\n## Analysis\n\n## Authorities\n\n## Open Questions\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/chronology.md"),
            "# Chronology\n\n| Date | Event | Source |\n|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/legal_memo.md"),
            "# Legal Memo Template\n\n## Issue\n\n## Brief Answer\n\n## Facts (Cited)\n\n## Analysis\n\n## Conclusion\n\n## Risk / Uncertainty\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/contract_issues.md"),
            "# Contract Issue List\n\n| Clause / Topic | Risk | Recommendation | Source |\n|---|---|---|---|\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/discovery_plan.md"),
            "# Discovery Plan\n\n## Custodians\n\n## Data Sources\n\n## Requests\n\n## Objections / Risks\n\n## Source Traceability\n".to_string(),
        ),
        (
            format!("{matter_prefix}/templates/research_synthesis.md"),
            "# Research Synthesis\n\n## Question Presented\n\n## Authorities Reviewed\n\n## Facts (Cited)\n\n## Analysis\n\n## Risk / Uncertainty\n".to_string(),
        ),
    ];

    let opened_at_ts =
        crate::channels::web::server::parse_optional_datetime("opened_date", opened_date.clone())?;
    let db_client = store
        .upsert_client_by_normalized_name(
            &state.user_id,
            &CreateClientParams {
                name: client.clone(),
                client_type: ClientType::Entity,
                email: None,
                phone: None,
                address: None,
                notes: None,
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    store
        .upsert_matter(
            &state.user_id,
            &UpsertMatterParams {
                matter_id: sanitized.clone(),
                client_id: db_client.id,
                status: MatterStatus::Active,
                stage: None,
                practice_area: practice_area.clone(),
                jurisdiction: jurisdiction.clone(),
                opened_at: opened_at_ts,
                closed_at: None,
                assigned_to: team.clone(),
                custom_fields: serde_json::json!({}),
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Seed conflict graph rows before filesystem writes so DB failures do not
    // leave behind an unindexed matter directory that cannot be retried.
    store
        .seed_matter_parties(&sanitized, &client, &adversaries, opened_date.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    crate::legal::matter::invalidate_conflict_cache();

    for (path, content) in scaffold {
        workspace
            .write(&path, &content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let value = serde_json::json!(sanitized);
    store
        .set_setting(
            &state.user_id,
            crate::channels::web::server::MATTER_ACTIVE_SETTING,
            &value,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "matter_created",
        state.user_id.as_str(),
        Some(sanitized.as_str()),
        AuditSeverity::Info,
        serde_json::json!({
            "matter_id": sanitized.clone(),
            "client_id": db_client.id.to_string(),
            "status": MatterStatus::Active.as_str(),
        }),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateMatterResponse {
            matter: MatterInfo {
                id: sanitized.clone(),
                client_id: Some(db_client.id.to_string()),
                client: Some(client),
                status: Some(MatterStatus::Active.as_str().to_string()),
                stage: None,
                confidentiality: Some(confidentiality),
                team,
                adversaries,
                retention: Some(retention),
                jurisdiction,
                practice_area,
                opened_date: opened_date.clone(),
                opened_at: opened_date,
            },
            active_matter_id: sanitized,
        }),
    ))
}

pub(crate) async fn matters_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
) -> Result<Json<MattersListResponse>, (StatusCode, String)> {
    // Only the matter owner can list all matters.
    if state.store.is_some() && principal.user_id != state.user_id {
        return Err((
            StatusCode::FORBIDDEN,
            "Insufficient permissions".to_string(),
        ));
    }
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    if let Some(store) = state.store.as_ref() {
        let matter_rows = store
            .list_matters_db(&state.user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut matters = Vec::with_capacity(matter_rows.len());
        for matter in matter_rows {
            matters.push(
                crate::channels::web::server::db_matter_to_info(state.as_ref(), matter).await,
            );
        }
        matters.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(Json(MattersListResponse { matters }));
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let entries = crate::channels::web::server::list_matters_root_entries(
        workspace.list(&matter_root).await,
    )?;
    let mut matters: Vec<MatterInfo> = Vec::new();
    for entry in entries.into_iter().filter(|entry| entry.is_directory) {
        let dir_name = entry.path.rsplit('/').next().unwrap_or("").to_string();
        if dir_name.is_empty() || dir_name == "_template" {
            continue;
        }
        let meta = crate::channels::web::server::read_workspace_matter_metadata_optional(
            Some(workspace),
            &matter_root,
            &dir_name,
        )
        .await;
        matters.push(MatterInfo {
            id: dir_name,
            client_id: None,
            client: meta.as_ref().map(|m| m.client.clone()),
            status: None,
            stage: None,
            confidentiality: meta.as_ref().map(|m| m.confidentiality.clone()),
            team: meta.as_ref().map(|m| m.team.clone()).unwrap_or_default(),
            adversaries: meta
                .as_ref()
                .map(|m| m.adversaries.clone())
                .unwrap_or_default(),
            retention: meta.as_ref().map(|m| m.retention.clone()),
            jurisdiction: meta.as_ref().and_then(|m| m.jurisdiction.clone()),
            practice_area: meta.as_ref().and_then(|m| m.practice_area.clone()),
            opened_date: meta.as_ref().and_then(|m| m.opened_date.clone()),
            opened_at: meta.as_ref().and_then(|m| m.opened_date.clone()),
        });
    }
    matters.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Json(MattersListResponse { matters }))
}

pub(crate) async fn clients_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ClientsQuery>,
) -> Result<Json<ClientsListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let clients = store
        .list_clients(&state.user_id, query.q.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::client_record_to_info)
        .collect();
    Ok(Json(ClientsListResponse { clients }))
}

pub(crate) async fn clients_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateClientRequest>,
) -> Result<(StatusCode, Json<ClientInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let name = crate::channels::web::server::parse_required_matter_field("name", &req.name)?;
    let client_type = crate::channels::web::server::parse_client_type(&req.client_type)?;
    let client = store
        .create_client(
            &state.user_id,
            &CreateClientParams {
                name,
                client_type,
                email: crate::channels::web::server::parse_optional_matter_field(req.email),
                phone: crate::channels::web::server::parse_optional_matter_field(req.phone),
                address: crate::channels::web::server::parse_optional_matter_field(req.address),
                notes: crate::channels::web::server::parse_optional_matter_field(req.notes),
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::client_record_to_info(client)),
    ))
}

pub(crate) async fn clients_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = crate::channels::web::server::parse_uuid(&id, "id")?;
    let client = store
        .get_client(&state.user_id, client_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;
    Ok(Json(crate::channels::web::server::client_record_to_info(
        client,
    )))
}

pub(crate) async fn clients_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateClientRequest>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = crate::channels::web::server::parse_uuid(&id, "id")?;
    let input = UpdateClientParams {
        name: req.name.map(|value| value.trim().to_string()),
        client_type: req
            .client_type
            .as_deref()
            .map(crate::channels::web::server::parse_client_type)
            .transpose()?,
        email: req.email.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        phone: req.phone.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        address: req.address.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        notes: req.notes.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
    };

    let client = store
        .update_client(&state.user_id, client_id, &input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;
    Ok(Json(crate::channels::web::server::client_record_to_info(
        client,
    )))
}

pub(crate) async fn clients_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = crate::channels::web::server::parse_uuid(&id, "id")?;
    let deleted = store
        .delete_client(&state.user_id, client_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Client not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_get_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
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
    let matter = store
        .get_matter_db(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;
    Ok(Json(
        crate::channels::web::server::db_matter_to_info(state.as_ref(), matter).await,
    ))
}

pub(crate) async fn matter_patch_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<UpdateMatterRequest>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
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
    let client_id = req
        .client_id
        .as_deref()
        .map(|value| crate::channels::web::server::parse_uuid(value, "client_id"))
        .transpose()?;
    if let Some(client_id) = client_id
        && store
            .get_client(&state.user_id, client_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "Client not found".to_string()));
    }
    let status = req
        .status
        .as_deref()
        .map(crate::channels::web::server::parse_matter_status)
        .transpose()?;

    let assigned_to = req
        .assigned_to
        .map(crate::channels::web::server::parse_matter_list);
    let custom_fields = if let Some(value) = req.custom_fields {
        if !value.is_object() {
            return Err((
                StatusCode::BAD_REQUEST,
                "'custom_fields' must be a JSON object".to_string(),
            ));
        }
        Some(value)
    } else {
        None
    };

    let input = UpdateMatterParams {
        client_id,
        status,
        stage: req.stage.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        practice_area: req.practice_area.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        jurisdiction: req.jurisdiction.map(|value| {
            value.and_then(|inner| {
                crate::channels::web::server::parse_optional_matter_field(Some(inner))
            })
        }),
        opened_at: crate::channels::web::server::parse_optional_datetime_patch(
            "opened_at",
            req.opened_at,
        )?,
        closed_at: crate::channels::web::server::parse_optional_datetime_patch(
            "closed_at",
            req.closed_at,
        )?,
        assigned_to,
        custom_fields,
    };

    let matter = store
        .update_matter(&state.user_id, &matter_id, &input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;

    if let Some(workspace) = state.workspace.as_ref() {
        let metadata_path = crate::channels::web::server::matter_metadata_path_for_gateway(
            state.as_ref(),
            &matter_id,
        );
        if let Ok(doc) = workspace.read(&metadata_path).await
            && let Ok(mut metadata) =
                serde_yml::from_str::<crate::legal::matter::MatterMetadata>(&doc.content)
        {
            metadata.matter_id = matter.matter_id.clone();
            metadata.team = matter.assigned_to.clone();
            metadata.jurisdiction = matter.jurisdiction.clone();
            metadata.practice_area = matter.practice_area.clone();
            metadata.opened_date = matter.opened_at.map(|dt| dt.date_naive().to_string());
            if let Ok(Some(client)) = store.get_client(&state.user_id, matter.client_id).await {
                metadata.client = client.name;
            }

            if let Ok(rendered) = serde_yml::to_string(&metadata) {
                let content = format!(
                    "# Matter metadata schema\n# Required: matter_id, client, confidentiality, retention\n{}",
                    rendered
                );
                if let Err(err) = workspace.write(&metadata_path, &content).await {
                    tracing::warn!(
                        matter_id = matter_id.as_str(),
                        "failed to sync matter.yaml after matter update: {}",
                        err
                    );
                }
            }
        }
    }

    if matches!(status, Some(MatterStatus::Closed)) {
        crate::channels::web::server::record_legal_audit_event(
            state.as_ref(),
            "matter_closed",
            state.user_id.as_str(),
            Some(matter_id.as_str()),
            AuditSeverity::Info,
            serde_json::json!({
                "matter_id": matter_id,
                "status": MatterStatus::Closed.as_str(),
            }),
        )
        .await;
    }

    Ok(Json(
        crate::channels::web::server::db_matter_to_info(state.as_ref(), matter).await,
    ))
}

pub(crate) async fn matter_delete_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let deleted = store
        .delete_matter(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    if let Some(active_value) = store
        .get_setting(
            &state.user_id,
            crate::channels::web::server::MATTER_ACTIVE_SETTING,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .and_then(|value| value.as_str().map(str::to_string))
        && crate::legal::policy::sanitize_matter_id(&active_value) == matter_id
    {
        store
            .delete_setting(
                &state.user_id,
                crate::channels::web::server::MATTER_ACTIVE_SETTING,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matters_active_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ActiveMatterResponse>, (StatusCode, String)> {
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let mut matter_id = if let Some(store) = state.store.as_ref() {
        store
            .get_setting(
                &state.user_id,
                crate::channels::web::server::MATTER_ACTIVE_SETTING,
            )
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .and_then(|v| v.as_str().map(crate::legal::policy::sanitize_matter_id))
    } else {
        None
    };

    if matter_id.as_deref().is_some_and(|id| id.is_empty()) {
        matter_id = None;
    }

    if let Some(ref candidate) = matter_id
        && let Some(workspace) = state.workspace.as_ref()
    {
        match crate::legal::matter::read_matter_metadata_for_root(
            workspace.as_ref(),
            &matter_root,
            candidate,
        )
        .await
        {
            Ok(_) => {}
            Err(crate::legal::matter::MatterMetadataValidationError::Missing { .. })
            | Err(crate::legal::matter::MatterMetadataValidationError::Invalid { .. }) => {
                matter_id = None;
            }
            Err(err @ crate::legal::matter::MatterMetadataValidationError::Storage { .. }) => {
                return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
            }
        }
    }

    Ok(Json(ActiveMatterResponse { matter_id }))
}

pub(crate) async fn matters_active_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SetActiveMatterRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());

    let trimmed = req
        .matter_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match trimmed {
        None => {
            store
                .delete_setting(
                    &state.user_id,
                    crate::channels::web::server::MATTER_ACTIVE_SETTING,
                )
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        Some(id) => {
            let workspace = state.workspace.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Workspace not available".to_string(),
            ))?;
            let sanitized = crate::legal::policy::sanitize_matter_id(id);
            if sanitized.is_empty() {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Matter ID is empty after sanitization".to_string(),
                ));
            }
            let metadata = match crate::legal::matter::read_matter_metadata_for_root(
                workspace.as_ref(),
                &matter_root,
                &sanitized,
            )
            .await
            {
                Ok(metadata) => metadata,
                Err(crate::legal::matter::MatterMetadataValidationError::Missing { path }) => {
                    return Err((
                        StatusCode::NOT_FOUND,
                        format!("Matter '{}' not found (missing '{}')", sanitized, path),
                    ));
                }
                Err(err @ crate::legal::matter::MatterMetadataValidationError::Invalid { .. }) => {
                    return Err((StatusCode::UNPROCESSABLE_ENTITY, err.to_string()));
                }
                Err(err @ crate::legal::matter::MatterMetadataValidationError::Storage { .. }) => {
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()));
                }
            };

            let checked_parties = build_checked_parties(&metadata.client, &metadata.adversaries);
            if !checked_parties.is_empty() {
                let mut hits = store
                    .find_conflict_hits_for_names(&checked_parties, 50)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                hits.retain(|hit| hit.matter_id != sanitized);

                if !hits.is_empty() {
                    let requested_decision = req.conflict_decision;
                    let requested_note = crate::channels::web::server::parse_optional_matter_field(
                        req.conflict_note,
                    );

                    if let Some(decision) = requested_decision {
                        persist_conflict_clearance_decision(
                            &state,
                            &sanitized,
                            decision,
                            requested_note,
                            None,
                            &hits,
                            "active_set_flow",
                        )
                        .await?;
                        if matches!(decision, ConflictDecision::Declined) {
                            return Err(active_set_conflict_declined_error(&hits));
                        }
                    } else {
                        let latest = store
                            .latest_conflict_clearance(&sanitized)
                            .await
                            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                        let matching_clearance = latest.as_ref().is_some_and(|clearance| {
                            clearance.hit_count == hits.len() as i32
                                && decision_allows_matter_activation(clearance.decision)
                                && clearance_signature_matches_hits(
                                    &sanitized,
                                    &hits,
                                    &clearance.hits_json,
                                )
                        });

                        if !matching_clearance {
                            if latest.as_ref().is_some_and(|clearance| {
                                clearance.hit_count == hits.len() as i32
                                    && matches!(clearance.decision, ConflictDecision::Declined)
                                    && clearance_signature_matches_hits(
                                        &sanitized,
                                        &hits,
                                        &clearance.hits_json,
                                    )
                            }) {
                                return Err(active_set_conflict_declined_error(&hits));
                            }
                            return Err(active_set_conflict_required_error(&hits));
                        }
                    }
                }
            }
            store
                .set_setting(
                    &state.user_id,
                    crate::channels::web::server::MATTER_ACTIVE_SETTING,
                    &serde_json::Value::String(sanitized),
                )
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_deadlines_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterDeadlinesResponse>, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let matter_prefix = format!("{matter_root}/{matter_id}");
    let deadlines = crate::channels::web::server::read_matter_deadlines_for_matter(
        state.as_ref(),
        &matter_id,
        &matter_prefix,
        chrono::Utc::now().date_naive(),
    )
    .await?;

    Ok(Json(MatterDeadlinesResponse {
        matter_id,
        deadlines,
    }))
}

fn court_rule_to_info(rule: &crate::legal::calendar::CourtRule) -> CourtRuleInfo {
    CourtRuleInfo {
        id: rule.id.clone(),
        citation: rule.citation.clone(),
        deadline_type: rule.deadline_type.as_str().to_string(),
        offset_days: rule.offset_days,
        court_days: rule.court_days,
        version: rule.version.clone(),
        jurisdiction: rule.jurisdiction.clone(),
    }
}

fn resolve_deadline_rule(
    deadline: &crate::db::MatterDeadlineRecord,
) -> Result<Option<crate::legal::calendar::CourtRule>, String> {
    if let Some(rule_id) = deadline
        .explanation
        .as_ref()
        .and_then(|value| value.get("rule_id"))
        .and_then(|value| value.as_str())
        && let Some(rule) = crate::legal::calendar::get_court_rule(rule_id)?
    {
        return Ok(Some(rule));
    }

    match deadline.rule_ref.as_deref() {
        Some(rule_ref) => crate::legal::calendar::find_court_rule_by_ref(rule_ref),
        None => Ok(None),
    }
}

async fn cascade_recompute_deadline_dependents(
    state: &GatewayState,
    matter_id: &str,
    trigger_deadline_id: uuid::Uuid,
    new_trigger_date: chrono::DateTime<chrono::Utc>,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };

    let mut visited = HashSet::new();
    let mut pending = vec![(trigger_deadline_id, new_trigger_date)];

    while let Some((current_deadline_id, current_trigger_date)) = pending.pop() {
        if !visited.insert(current_deadline_id) {
            continue;
        }

        let dependents = store
            .list_deadlines_by_trigger(&state.user_id, matter_id, current_deadline_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

        for dep in dependents {
            let Some(rule) = resolve_deadline_rule(&dep)
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
            else {
                tracing::warn!(
                    "skipping cascade recompute for dependent deadline {} without a resolvable rule",
                    dep.id
                );
                continue;
            };

            let (new_params, trace) = crate::legal::calendar::deadline_from_rule_with_trace(
                &dep.title,
                &rule,
                current_trigger_date,
                dep.reminder_days.clone(),
                dep.computed_from,
                dep.task_id,
            );
            let new_exp = serde_json::to_value(&trace).unwrap_or(serde_json::Value::Null);

            let updated = match store
                .update_matter_deadline(
                    &state.user_id,
                    matter_id,
                    dep.id,
                    &UpdateMatterDeadlineParams {
                        title: None,
                        deadline_type: None,
                        due_at: Some(new_params.due_at),
                        completed_at: None,
                        reminder_days: None,
                        rule_ref: None,
                        computed_from: None,
                        task_id: None,
                        explanation: Some(Some(new_exp)),
                        rule_version: Some(Some(rule.version.clone())),
                        is_unsupported: None,
                    },
                )
                .await
            {
                Ok(Some(updated)) => updated,
                Ok(None) => continue,
                Err(err) => {
                    tracing::warn!(
                        "cascade recompute failed for dependent deadline {}: {}",
                        dep.id,
                        err
                    );
                    continue;
                }
            };

            if let Err(err) =
                crate::channels::web::server::sync_deadline_reminder_routines_for_record(
                    state, &updated,
                )
                .await
            {
                tracing::warn!(
                    "failed to sync reminder routines after cascade recompute for deadline {}: {:?}",
                    updated.id,
                    err
                );
            }

            pending.push((updated.id, updated.due_at));
        }
    }

    Ok(())
}

pub(crate) async fn matter_deadlines_create_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterDeadlineRequest>,
) -> Result<(StatusCode, Json<MatterDeadlineRecordInfo>), (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;

    let title = req.title.trim();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'title' is required".to_string()));
    }
    let deadline_type =
        crate::channels::web::server::parse_matter_deadline_type(&req.deadline_type)?;
    let due_at = crate::channels::web::server::parse_datetime_value("due_at", &req.due_at)?;
    let completed_at =
        crate::channels::web::server::parse_optional_datetime("completed_at", req.completed_at)?;
    let reminder_days = crate::channels::web::server::normalize_reminder_days(&req.reminder_days)?;
    let rule_ref = crate::channels::web::server::parse_optional_matter_field(req.rule_ref);
    crate::channels::web::server::validate_optional_matter_field_length("rule_ref", &rule_ref)?;
    let computed_from = crate::channels::web::server::parse_optional_uuid_field(
        req.computed_from,
        "computed_from",
    )?;
    let task_id = crate::channels::web::server::parse_optional_uuid_field(req.task_id, "task_id")?;

    // If rule_ref is a known rule ID and computed_from is set, auto-compute the
    // explanation trace from the trigger deadline so it is stored with the record.
    let (explanation, rule_version) = if let (Some(rule_id), Some(trigger_deadline_id)) =
        (&rule_ref, computed_from)
    {
        match crate::legal::calendar::find_court_rule_by_ref(rule_id) {
            Ok(Some(ref rule)) => {
                // Fetch the trigger deadline's due_at to use as the trigger date.
                match store
                    .get_matter_deadline(&state.user_id, &matter_id, trigger_deadline_id)
                    .await
                {
                    Ok(Some(trigger)) => {
                        let (_, trace) = crate::legal::calendar::deadline_from_rule_with_trace(
                            title,
                            rule,
                            trigger.due_at,
                            reminder_days.clone(),
                            computed_from,
                            task_id,
                        );
                        let exp = serde_json::to_value(&trace).unwrap_or(serde_json::Value::Null);
                        (Some(exp), Some(rule.version.clone()))
                    }
                    _ => (None, None),
                }
            }
            _ => (None, None),
        }
    } else {
        (None, None)
    };

    let is_unsupported = req.is_unsupported.unwrap_or(false);

    let created = store
        .create_matter_deadline(
            &state.user_id,
            &matter_id,
            &CreateMatterDeadlineParams {
                title: title.to_string(),
                deadline_type,
                due_at,
                completed_at,
                reminder_days,
                rule_ref,
                computed_from,
                task_id,
                explanation,
                rule_version,
                is_unsupported,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    crate::channels::web::server::sync_deadline_reminder_routines_for_record(
        state.as_ref(),
        &created,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::deadline_record_to_info(
            created,
        )),
    ))
}

pub(crate) async fn matter_deadlines_patch_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, deadline_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterDeadlineRequest>,
) -> Result<Json<MatterDeadlineRecordInfo>, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;
    let deadline_id = crate::channels::web::server::parse_uuid(deadline_id.trim(), "deadline_id")?;

    let title = req.title.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let deadline_type = req
        .deadline_type
        .as_deref()
        .map(crate::channels::web::server::parse_matter_deadline_type)
        .transpose()?;
    let due_at = req
        .due_at
        .as_deref()
        .map(|value| crate::channels::web::server::parse_datetime_value("due_at", value))
        .transpose()?;
    let completed_at = crate::channels::web::server::parse_optional_datetime_patch(
        "completed_at",
        req.completed_at,
    )?;
    let reminder_days = req
        .reminder_days
        .as_ref()
        .map(|values| crate::channels::web::server::normalize_reminder_days(values))
        .transpose()?;
    let rule_ref = crate::channels::web::server::parse_optional_matter_field_patch(req.rule_ref);
    if let Some(Some(ref value)) = rule_ref {
        crate::channels::web::server::validate_optional_matter_field_length(
            "rule_ref",
            &Some(value.clone()),
        )?;
    }
    let computed_from = crate::channels::web::server::parse_optional_uuid_patch_field(
        req.computed_from,
        "computed_from",
    )?;
    let task_id =
        crate::channels::web::server::parse_optional_uuid_patch_field(req.task_id, "task_id")?;

    let is_unsupported = req.is_unsupported;

    let updated = store
        .update_matter_deadline(
            &state.user_id,
            &matter_id,
            deadline_id,
            &UpdateMatterDeadlineParams {
                title,
                deadline_type,
                due_at,
                completed_at,
                reminder_days,
                rule_ref,
                computed_from,
                task_id,
                explanation: None,
                rule_version: None,
                is_unsupported,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Deadline not found".to_string()))?;

    crate::channels::web::server::sync_deadline_reminder_routines_for_record(
        state.as_ref(),
        &updated,
    )
    .await?;

    // Cascade recompute: if due_at changed, update any deadlines computed from this one.
    if let Some(new_trigger_date) = due_at {
        cascade_recompute_deadline_dependents(
            state.as_ref(),
            &matter_id,
            deadline_id,
            new_trigger_date,
        )
        .await?;
    }

    Ok(Json(crate::channels::web::server::deadline_record_to_info(
        updated,
    )))
}

pub(crate) async fn matter_deadlines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, deadline_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;
    let deadline_id = crate::channels::web::server::parse_uuid(deadline_id.trim(), "deadline_id")?;

    let existing = store
        .get_matter_deadline(&state.user_id, &matter_id, deadline_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Deadline not found".to_string()))?;

    let deleted = store
        .delete_matter_deadline(&state.user_id, &matter_id, deadline_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Deadline not found".to_string()));
    }

    crate::channels::web::server::disable_deadline_reminder_routines(
        state.as_ref(),
        &existing.matter_id,
        existing.id,
    )
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_deadlines_compute_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
    Json(req): Json<MatterDeadlineComputeRequest>,
) -> Result<Json<MatterDeadlineComputeResponse>, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    // Saving a deadline requires Collaborator access; read-only compute only needs Viewer.
    let minimum_role = if req.save {
        MatterMemberRole::Collaborator
    } else {
        MatterMemberRole::Viewer
    };
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        minimum_role,
    )
    .await
    .map_err(|s| (s, String::new()))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    let rule_id = req.rule_id.trim();
    if rule_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'rule_id' is required".to_string()));
    }

    let rule = crate::legal::calendar::get_court_rule(rule_id)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err))?
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!("Unknown rule_id '{rule_id}'"),
        ))?;
    let trigger =
        crate::channels::web::server::parse_datetime_value("trigger_date", &req.trigger_date)?;
    let reminder_days = crate::channels::web::server::normalize_reminder_days(&req.reminder_days)?;
    let computed_from = crate::channels::web::server::parse_optional_uuid_field(
        req.computed_from,
        "computed_from",
    )?;
    let task_id = crate::channels::web::server::parse_optional_uuid_field(req.task_id, "task_id")?;
    let title = crate::channels::web::server::parse_optional_matter_field(req.title)
        .unwrap_or_else(|| format!("{} deadline", rule.citation));

    let (computed, trace) = crate::legal::calendar::deadline_from_rule_with_trace(
        &title,
        &rule,
        trigger,
        reminder_days,
        computed_from,
        task_id,
    );
    let explanation = serde_json::to_value(&trace).unwrap_or(serde_json::Value::Null);

    // Optionally persist the computed deadline to the database.
    let saved = if req.save {
        let store = state.store.as_ref().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Database not available".to_string(),
        ))?;
        crate::channels::web::server::ensure_matter_db_row_from_workspace(
            state.as_ref(),
            &matter_id,
        )
        .await?;
        let record = store
            .create_matter_deadline(&state.user_id, &matter_id, &computed)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        crate::channels::web::server::sync_deadline_reminder_routines_for_record(
            state.as_ref(),
            &record,
        )
        .await?;
        Some(crate::channels::web::server::deadline_record_to_info(
            record,
        ))
    } else {
        None
    };

    Ok(Json(MatterDeadlineComputeResponse {
        matter_id,
        rule: court_rule_to_info(&rule),
        deadline: crate::channels::web::server::deadline_compute_preview_from_params(
            &computed,
            explanation,
        ),
        saved,
    }))
}

// ==================== Deadline Override / Audit Handlers ====================

/// `POST /api/matters/{id}/deadlines/{deadline_id}/override` — apply a manual override (Collaborator+).
///
/// Writes an immutable row to `deadline_override_audit` and updates the deadline's `due_at`,
/// `is_manual_override`, `override_reason`, `override_by`, and `overridden_at`.
pub(crate) async fn matter_deadline_override_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, deadline_id)): Path<(String, String)>,
    Json(req): Json<DeadlineOverrideRequest>,
) -> Result<Json<MatterDeadlineRecordInfo>, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Collaborator,
    )
    .await
    .map_err(|s| (s, String::new()))?;

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;

    let deadline_id = crate::channels::web::server::parse_uuid(deadline_id.trim(), "deadline_id")?;
    let new_due_at = crate::channels::web::server::parse_datetime_value("due_at", &req.due_at)?;

    let reason = req.reason.trim().to_string();
    if reason.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'reason' is required".to_string()));
    }

    let updated = store
        .apply_deadline_override(
            &state.user_id,
            &matter_id,
            deadline_id,
            &OverrideDeadlineParams {
                new_due_at,
                reason,
                overriding_user_id: principal.user_id.clone(),
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    crate::channels::web::server::sync_deadline_reminder_routines_for_record(
        state.as_ref(),
        &updated,
    )
    .await?;

    cascade_recompute_deadline_dependents(state.as_ref(), &matter_id, deadline_id, updated.due_at)
        .await?;

    Ok(Json(crate::channels::web::server::deadline_record_to_info(
        updated,
    )))
}

/// `GET /api/matters/{id}/deadlines/{deadline_id}/audit` — list override audit trail (Viewer+).
pub(crate) async fn matter_deadline_audit_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, deadline_id)): Path<(String, String)>,
) -> Result<Json<DeadlineOverrideAuditResponse>, (StatusCode, String)> {
    let sanitized_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &sanitized_id,
        &principal.user_id,
        MatterMemberRole::Viewer,
    )
    .await
    .map_err(|s| (s, String::new()))?;

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id = crate::channels::web::server::ensure_existing_matter_for_route(
        workspace.as_ref(),
        &matter_root,
        &id,
    )
    .await?;

    let deadline_id = crate::channels::web::server::parse_uuid(deadline_id.trim(), "deadline_id")?;

    let entries = store
        .list_deadline_override_audit(&state.user_id, &matter_id, deadline_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let audit_infos = entries
        .into_iter()
        .map(|e| DeadlineOverrideAuditInfo {
            id: e.id.to_string(),
            deadline_id: e.deadline_id.to_string(),
            user_id: e.user_id,
            previous_due_at: e.previous_due_at.to_rfc3339(),
            new_due_at: e.new_due_at.to_rfc3339(),
            reason: e.reason,
            created_at: e.created_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(DeadlineOverrideAuditResponse {
        deadline_id: deadline_id.to_string(),
        entries: audit_infos,
    }))
}

// ==================== Membership Handlers ====================

/// `GET /api/matters/{id}/members` — list members (Owner only).
async fn matter_members_list_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(id): Path<String>,
) -> Result<Json<MatterMembersListResponse>, StatusCode> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store
        .list_matter_memberships(&state.user_id, &matter_id)
        .await
        .map_err(|e| {
            tracing::error!("list_matter_memberships failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    let members = rows
        .into_iter()
        .map(|r| MatterMemberResponse {
            user_id: r.member_user_id,
            role: r.role.as_str().to_string(),
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();
    Ok(Json(MatterMembersListResponse { matter_id, members }))
}

/// `PUT /api/matters/{id}/members/{member_user_id}` — add or update a member (Owner only).
async fn matter_member_upsert_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, member_user_id)): Path<(String, String)>,
    Json(body): Json<UpsertMatterMemberRequest>,
) -> Result<Json<MatterMemberResponse>, StatusCode> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await?;
    // Owner cannot be assigned via this endpoint — ownership is implicit.
    if body.role.trim().eq_ignore_ascii_case("owner") {
        return Err(StatusCode::BAD_REQUEST);
    }
    let role = MatterMemberRole::from_db_value(&body.role).ok_or(StatusCode::BAD_REQUEST)?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let record = store
        .upsert_matter_membership(&UpsertMatterMembershipParams {
            matter_owner_user_id: state.user_id.clone(),
            matter_id,
            member_user_id,
            role,
        })
        .await
        .map_err(|e| {
            let msg = e.to_string();
            // FK violation means the supplied member_user_id does not exist in the users table.
            if msg.contains("FOREIGN KEY constraint") || msg.contains("foreign key constraint") {
                StatusCode::UNPROCESSABLE_ENTITY
            } else {
                tracing::error!("upsert_matter_membership failed: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            }
        })?;
    Ok(Json(MatterMemberResponse {
        user_id: record.member_user_id,
        role: record.role.as_str().to_string(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }))
}

/// `DELETE /api/matters/{id}/members/{member_user_id}` — remove a member (Owner only).
async fn matter_member_remove_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path((id, member_user_id)): Path<(String, String)>,
) -> Result<StatusCode, StatusCode> {
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    require_matter_access(
        &state.store,
        &state.user_id,
        &matter_id,
        &principal.user_id,
        MatterMemberRole::Owner,
    )
    .await?;
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .remove_matter_membership(&state.user_id, &matter_id, &member_user_id)
        .await
        .map_err(|e| {
            tracing::error!("remove_matter_membership failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(StatusCode::NO_CONTENT)
}
