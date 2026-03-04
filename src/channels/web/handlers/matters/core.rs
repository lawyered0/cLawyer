//! Matter and client core handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    AuditSeverity, CreateClientParams, CreateMatterDeadlineParams, MatterStatus,
    UpdateClientParams, UpdateMatterDeadlineParams, UpdateMatterParams,
};

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ClientsQuery {
    pub(crate) q: Option<String>,
}

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/matters", get(matters_list_handler))
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
}

pub(crate) async fn matters_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MattersListResponse>, (StatusCode, String)> {
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    if let Some(store) = state.store.as_ref() {
        let matter_rows = store
            .list_matters_db(&state.user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut matters = Vec::with_capacity(matter_rows.len());
        for matter in matter_rows {
            matters.push(crate::channels::web::server::db_matter_to_info(state.as_ref(), matter).await);
        }
        matters.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(Json(MattersListResponse { matters }));
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let entries = crate::channels::web::server::list_matters_root_entries(workspace.list(&matter_root).await)?;
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
    Ok(Json(crate::channels::web::server::client_record_to_info(client)))
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
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
        }),
        phone: req.phone.map(|value| {
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
        }),
        address: req.address.map(|value| {
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
        }),
        notes: req.notes.map(|value| {
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
        }),
    };

    let client = store
        .update_client(&state.user_id, client_id, &input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;
    Ok(Json(crate::channels::web::server::client_record_to_info(client)))
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
    Path(id): Path<String>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    let matter = store
        .get_matter_db(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;
    Ok(Json(crate::channels::web::server::db_matter_to_info(
        state.as_ref(),
        matter,
    )
    .await))
}

pub(crate) async fn matter_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMatterRequest>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
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

    let assigned_to = req.assigned_to.map(crate::channels::web::server::parse_matter_list);
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
        stage: req
            .stage
            .map(|value| value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))),
        practice_area: req.practice_area.map(|value| {
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
        }),
        jurisdiction: req.jurisdiction.map(|value| {
            value.and_then(|inner| crate::channels::web::server::parse_optional_matter_field(Some(inner)))
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
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
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

    let trimmed = req.matter_id.as_deref().map(str::trim).filter(|s| !s.is_empty());

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
            match crate::legal::matter::read_matter_metadata_for_root(
                workspace.as_ref(),
                &matter_root,
                &sanitized,
            )
            .await
            {
                Ok(_) => {}
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
    Path(id): Path<String>,
) -> Result<Json<MatterDeadlinesResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id =
        crate::channels::web::server::ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id)
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
    }
}

pub(crate) async fn matter_deadlines_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterDeadlineRequest>,
) -> Result<(StatusCode, Json<MatterDeadlineRecordInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id =
        crate::channels::web::server::ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id)
            .await?;
    crate::channels::web::server::ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id)
        .await?;

    let title = req.title.trim();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'title' is required".to_string()));
    }
    let deadline_type = crate::channels::web::server::parse_matter_deadline_type(&req.deadline_type)?;
    let due_at = crate::channels::web::server::parse_datetime_value("due_at", &req.due_at)?;
    let completed_at =
        crate::channels::web::server::parse_optional_datetime("completed_at", req.completed_at)?;
    let reminder_days = crate::channels::web::server::normalize_reminder_days(&req.reminder_days)?;
    let rule_ref = crate::channels::web::server::parse_optional_matter_field(req.rule_ref);
    crate::channels::web::server::validate_optional_matter_field_length("rule_ref", &rule_ref)?;
    let computed_from =
        crate::channels::web::server::parse_optional_uuid_field(req.computed_from, "computed_from")?;
    let task_id = crate::channels::web::server::parse_optional_uuid_field(req.task_id, "task_id")?;

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
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    crate::channels::web::server::sync_deadline_reminder_routines_for_record(state.as_ref(), &created)
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::deadline_record_to_info(created)),
    ))
}

pub(crate) async fn matter_deadlines_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, deadline_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterDeadlineRequest>,
) -> Result<Json<MatterDeadlineRecordInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id =
        crate::channels::web::server::ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id)
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
    let completed_at =
        crate::channels::web::server::parse_optional_datetime_patch("completed_at", req.completed_at)?;
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
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Deadline not found".to_string()))?;

    crate::channels::web::server::sync_deadline_reminder_routines_for_record(state.as_ref(), &updated)
        .await?;

    Ok(Json(crate::channels::web::server::deadline_record_to_info(
        updated,
    )))
}

pub(crate) async fn matter_deadlines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, deadline_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id =
        crate::channels::web::server::ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id)
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
    Path(id): Path<String>,
    Json(req): Json<MatterDeadlineComputeRequest>,
) -> Result<Json<MatterDeadlineComputeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = crate::channels::web::server::matter_root_for_gateway(state.as_ref());
    let matter_id =
        crate::channels::web::server::ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id)
            .await?;
    let rule_id = req.rule_id.trim();
    if rule_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'rule_id' is required".to_string()));
    }

    let rule = crate::legal::calendar::get_court_rule(rule_id)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err))?
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!("Unknown rule_id '{}'", rule_id),
        ))?;
    let trigger = crate::channels::web::server::parse_datetime_value("trigger_date", &req.trigger_date)?;
    let reminder_days = crate::channels::web::server::normalize_reminder_days(&req.reminder_days)?;
    let computed_from =
        crate::channels::web::server::parse_optional_uuid_field(req.computed_from, "computed_from")?;
    let task_id = crate::channels::web::server::parse_optional_uuid_field(req.task_id, "task_id")?;
    let title = crate::channels::web::server::parse_optional_matter_field(req.title)
        .unwrap_or_else(|| format!("{} deadline", rule.citation));

    let computed = crate::legal::calendar::deadline_from_rule(
        &title,
        &rule,
        trigger,
        reminder_days,
        computed_from,
        task_id,
    );

    Ok(Json(MatterDeadlineComputeResponse {
        matter_id,
        rule: court_rule_to_info(&rule),
        deadline: crate::channels::web::server::deadline_compute_preview_from_params(&computed),
    }))
}
