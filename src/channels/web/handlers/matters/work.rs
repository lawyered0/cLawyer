//! Matter workstream handlers (tasks and notes).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::{
    CreateMatterNoteParams, CreateMatterTaskParams, MatterTaskStatus, UpdateMatterNoteParams,
    UpdateMatterTaskParams,
};

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route(
            "/api/matters/{id}/tasks",
            get(matter_tasks_list_handler).post(matter_tasks_create_handler),
        )
        .route(
            "/api/matters/{id}/tasks/{task_id}",
            axum::routing::patch(matter_tasks_patch_handler).delete(matter_tasks_delete_handler),
        )
        .route(
            "/api/matters/{id}/notes",
            get(matter_notes_list_handler).post(matter_notes_create_handler),
        )
        .route(
            "/api/matters/{id}/notes/{note_id}",
            axum::routing::patch(matter_notes_patch_handler).delete(matter_notes_delete_handler),
        )
}

pub(crate) async fn matter_tasks_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTasksListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let tasks = store
        .list_matter_tasks(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::matter_task_record_to_info)
        .collect();
    Ok(Json(MatterTasksListResponse { tasks }))
}

pub(crate) async fn matter_tasks_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterTaskRequest>,
) -> Result<(StatusCode, Json<MatterTaskInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let title = crate::channels::web::server::parse_required_matter_field("title", &req.title)?;
    let status = req
        .status
        .as_deref()
        .map(crate::channels::web::server::parse_matter_task_status)
        .transpose()?
        .unwrap_or(MatterTaskStatus::Todo);
    let due_at = crate::channels::web::server::parse_optional_datetime("due_at", req.due_at)?;
    let blocked_by = crate::channels::web::server::parse_uuid_list(&req.blocked_by, "blocked_by")?;
    let task = store
        .create_matter_task(
            &state.user_id,
            &matter_id,
            &CreateMatterTaskParams {
                title,
                description: crate::channels::web::server::parse_optional_matter_field(
                    req.description,
                ),
                status,
                assignee: crate::channels::web::server::parse_optional_matter_field(req.assignee),
                due_at,
                blocked_by,
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::matter_task_record_to_info(
            task,
        )),
    ))
}

pub(crate) async fn matter_tasks_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, task_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterTaskRequest>,
) -> Result<Json<MatterTaskInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let task_id = crate::channels::web::server::parse_uuid(&task_id, "task_id")?;
    let status = req
        .status
        .as_deref()
        .map(crate::channels::web::server::parse_matter_task_status)
        .transpose()?;
    let blocked_by = req
        .blocked_by
        .as_ref()
        .map(|values| crate::channels::web::server::parse_uuid_list(values, "blocked_by"))
        .transpose()?;
    let due_at = crate::channels::web::server::parse_optional_datetime_patch("due_at", req.due_at)?;
    let task = store
        .update_matter_task(
            &state.user_id,
            &matter_id,
            task_id,
            &UpdateMatterTaskParams {
                title: req.title.map(|value| value.trim().to_string()),
                description: req.description.map(|value| {
                    value.and_then(|inner| {
                        crate::channels::web::server::parse_optional_matter_field(Some(inner))
                    })
                }),
                status,
                assignee: req.assignee.map(|value| {
                    value.and_then(|inner| {
                        crate::channels::web::server::parse_optional_matter_field(Some(inner))
                    })
                }),
                due_at,
                blocked_by,
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Task not found".to_string()))?;
    Ok(Json(
        crate::channels::web::server::matter_task_record_to_info(task),
    ))
}

pub(crate) async fn matter_tasks_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, task_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let task_id = crate::channels::web::server::parse_uuid(&task_id, "task_id")?;
    let deleted = store
        .delete_matter_task(&state.user_id, &matter_id, task_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Task not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn matter_notes_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterNotesListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let notes = store
        .list_matter_notes(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .into_iter()
        .map(crate::channels::web::server::matter_note_record_to_info)
        .collect();
    Ok(Json(MatterNotesListResponse { notes }))
}

pub(crate) async fn matter_notes_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterNoteRequest>,
) -> Result<(StatusCode, Json<MatterNoteInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let author = crate::channels::web::server::parse_required_matter_field("author", &req.author)?;
    let body = crate::channels::web::server::parse_required_matter_field("body", &req.body)?;
    let note = store
        .create_matter_note(
            &state.user_id,
            &matter_id,
            &CreateMatterNoteParams {
                author,
                body,
                pinned: req.pinned,
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(crate::channels::web::server::matter_note_record_to_info(
            note,
        )),
    ))
}

pub(crate) async fn matter_notes_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, note_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterNoteRequest>,
) -> Result<Json<MatterNoteInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let note_id = crate::channels::web::server::parse_uuid(&note_id, "note_id")?;
    let note = store
        .update_matter_note(
            &state.user_id,
            &matter_id,
            note_id,
            &UpdateMatterNoteParams {
                author: req.author.map(|value| value.trim().to_string()),
                body: req.body.map(|value| value.trim().to_string()),
                pinned: req.pinned,
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Note not found".to_string()))?;
    Ok(Json(
        crate::channels::web::server::matter_note_record_to_info(note),
    ))
}

pub(crate) async fn matter_notes_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, note_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = crate::channels::web::server::sanitize_matter_id_for_route(&id)?;
    crate::channels::web::server::ensure_existing_matter_db(state.as_ref(), &matter_id).await?;
    let note_id = crate::channels::web::server::parse_uuid(&note_id, "note_id")?;
    let deleted = store
        .delete_matter_note(&state.user_id, &matter_id, note_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Note not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}
