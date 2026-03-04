//! Chat handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::json;
use uuid::Uuid;

use crate::channels::IncomingMessage;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
}

pub(crate) async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    if !state.chat_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let mut msg = IncomingMessage::new("gateway", &state.user_id, &req.content);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
    }
    msg = msg.with_metadata(
        crate::channels::web::server::build_chat_message_metadata(
            state.as_ref(),
            req.thread_id.as_deref(),
        )
        .await,
    );

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    let (approved, always) = match req.action.as_str() {
        "approve" => (true, false),
        "always" => (true, true),
        "deny" => (false, false),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown action: {}", other),
            ));
        }
    };

    let request_id = Uuid::parse_str(&req.request_id).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid request_id (expected UUID)".to_string(),
        )
    })?;

    let approval = crate::agent::submission::Submission::ExecApproval {
        request_id,
        approved,
        always,
    };
    let content = serde_json::to_string(&approval).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize approval: {}", e),
        )
    })?;

    let mut msg = IncomingMessage::new("gateway", &state.user_id, content);

    if let Some(ref thread_id) = req.thread_id {
        msg = msg.with_thread(thread_id);
    }
    msg = msg.with_metadata(
        crate::channels::web::server::build_chat_message_metadata(
            state.as_ref(),
            req.thread_id.as_deref(),
        )
        .await,
    );

    let msg_id = msg.id;

    let tx_guard = state.msg_tx.read().await;
    let tx = tx_guard.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Channel not started".to_string(),
    ))?;

    tx.send(msg).await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Channel closed".to_string(),
        )
    })?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SendMessageResponse {
            message_id: msg_id,
            status: "accepted",
        }),
    ))
}

pub(crate) async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Extension manager not available".to_string(),
    ))?;

    let result = ext_mgr
        .auth(&req.extension_name, Some(&req.token))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if result.status == "authenticated" {
        let msg = match ext_mgr.activate(&req.extension_name).await {
            Ok(r) => format!(
                "{} authenticated ({} tools loaded)",
                req.extension_name,
                r.tools_loaded.len()
            ),
            Err(e) => format!(
                "{} authenticated but activation failed: {}",
                req.extension_name, e
            ),
        };

        crate::channels::web::server::clear_auth_mode(&state).await;

        state.sse.broadcast(SseEvent::AuthCompleted {
            extension_name: req.extension_name,
            success: true,
            message: msg.clone(),
        });

        Ok(Json(ActionResponse::ok(msg)))
    } else {
        state.sse.broadcast(SseEvent::AuthRequired {
            extension_name: req.extension_name.clone(),
            instructions: result.instructions.clone(),
            auth_url: result.auth_url.clone(),
            setup_url: result.setup_url.clone(),
        });
        Ok(Json(ActionResponse::fail(
            result
                .instructions
                .unwrap_or_else(|| "Invalid token".to_string()),
        )))
    }
}

pub(crate) async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Json(_req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    crate::channels::web::server::clear_auth_mode(&state).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
}

pub(crate) async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let sse = state.sse.subscribe().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Too many connections".to_string(),
    ))?;
    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        sse,
    ))
}

pub(crate) async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    let host = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .and_then(|rest| rest.split(':').next()?.split('/').next())
        .unwrap_or("");

    let is_local = matches!(host, "localhost" | "127.0.0.1" | "[::1]");
    if !is_local {
        return Err((
            StatusCode::FORBIDDEN,
            "WebSocket origin not allowed".to_string(),
        ));
    }
    Ok(ws.on_upgrade(move |socket| crate::channels::web::ws::handle_ws_connection(socket, state)))
}

pub(crate) async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<crate::channels::web::server::HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&state.user_id).await;
    let limit = query
        .limit
        .unwrap_or(crate::channels::web::server::CHAT_HISTORY_DEFAULT_LIMIT);
    if !(crate::channels::web::server::CHAT_HISTORY_MIN_LIMIT
        ..=crate::channels::web::server::CHAT_HISTORY_MAX_LIMIT)
        .contains(&limit)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "'limit' must be between 1 and 200".to_string(),
        ));
    }

    let before_cursor = query
        .before
        .as_deref()
        .map(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        "Invalid 'before' timestamp".to_string(),
                    )
                })
        })
        .transpose()?;

    let (thread_id, thread_exists_in_memory, in_memory_turns) = {
        let sess = session.lock().await;
        let thread_id = if let Some(ref tid) = query.thread_id {
            Uuid::parse_str(tid)
                .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
        } else {
            sess.active_thread
                .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
        };
        let in_memory_turns = if before_cursor.is_none() {
            sess.threads
                .get(&thread_id)
                .filter(|thread| !thread.turns.is_empty())
                .map(crate::channels::web::server::build_turns_from_session_thread)
        } else {
            None
        };
        (
            thread_id,
            sess.threads.contains_key(&thread_id),
            in_memory_turns,
        )
    };

    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &state.user_id)
            .await
            .unwrap_or(false);
        if !owned && !thread_exists_in_memory {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
        }
    }

    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = crate::channels::web::server::build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
        }));
    }

    if let Some(turns) = in_memory_turns {
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
        }));
    }

    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = crate::channels::web::server::build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
            }));
        }
    }

    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
    }))
}

pub(crate) async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<crate::channels::web::server::ThreadListQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let matter_filter = query
        .matter_id
        .as_deref()
        .and_then(crate::legal::policy::sanitize_optional_matter_id);
    if query.matter_id.as_deref().is_some() && matter_filter.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'matter_id' is empty after sanitization".to_string(),
        ));
    }

    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&state.user_id).await;
    let active_thread = {
        let sess = session.lock().await;
        sess.active_thread
    };

    if let Some(ref store) = state.store {
        let assistant_id = if matter_filter.is_none() {
            Some(
                store
                    .get_or_create_assistant_conversation(&state.user_id, "gateway")
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?,
            )
        } else {
            None
        };

        let summaries_result = if let Some(ref matter_id) = matter_filter {
            store.list_conversations_with_preview_for_matter(
                &state.user_id,
                "gateway",
                matter_id,
                50,
            )
        } else {
            store.list_conversations_with_preview(&state.user_id, "gateway", 50)
        };

        match summaries_result.await {
            Ok(summaries) => {
                let mut assistant_thread = None;
                let mut threads = Vec::new();

                for summary in summaries {
                    let info = ThreadInfo {
                        id: summary.id,
                        state: "Idle".to_string(),
                        turn_count: (summary.message_count / 2).max(0) as usize,
                        created_at: summary.started_at.to_rfc3339(),
                        updated_at: summary.last_activity.to_rfc3339(),
                        title: summary.title,
                        matter_id: summary.matter_id,
                        thread_type: summary.thread_type,
                    };

                    if assistant_id.is_some_and(|id| summary.id == id) {
                        assistant_thread = Some(info);
                    } else {
                        threads.push(info);
                    }
                }

                if assistant_thread.is_none()
                    && let Some(id) = assistant_id
                {
                    assistant_thread = Some(ThreadInfo {
                        id,
                        state: "Idle".to_string(),
                        turn_count: 0,
                        created_at: chrono::Utc::now().to_rfc3339(),
                        updated_at: chrono::Utc::now().to_rfc3339(),
                        title: None,
                        matter_id: None,
                        thread_type: Some("assistant".to_string()),
                    });
                }

                return Ok(Json(ThreadListResponse {
                    assistant_thread,
                    threads,
                    active_thread,
                }));
            }
            Err(err) => {
                tracing::warn!(
                    "Falling back to in-memory thread list after DB query error: {}",
                    err
                );
            }
        }
    }

    let sess = session.lock().await;
    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .filter(|thread| {
            if let Some(ref filter) = matter_filter {
                crate::legal::policy::matter_id_from_metadata(&thread.metadata)
                    .as_ref()
                    .is_some_and(|matter_id| matter_id == filter)
            } else {
                true
            }
        })
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            matter_id: crate::legal::policy::matter_id_from_metadata(&t.metadata),
            thread_type: None,
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

pub(crate) async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let active_matter =
        crate::channels::web::server::load_active_matter_for_chat(state.as_ref()).await;
    let session = session_manager.get_or_create_session(&state.user_id).await;
    let (thread_id, state_label, turn_count, created_at, updated_at) = {
        let mut sess = session.lock().await;
        let thread = sess.create_thread();
        if let Some(ref matter_id) = active_matter {
            thread.metadata = json!({ "matter_id": matter_id });
        }
        (
            thread.id,
            format!("{:?}", thread.state),
            thread.turns.len(),
            thread.created_at.to_rfc3339(),
            thread.updated_at.to_rfc3339(),
        )
    };
    let info = ThreadInfo {
        id: thread_id,
        state: state_label,
        turn_count,
        created_at,
        updated_at,
        title: None,
        matter_id: active_matter.clone(),
        thread_type: Some("thread".to_string()),
    };

    if let Some(ref store) = state.store {
        if let Err(e) = store
            .ensure_conversation(thread_id, "gateway", &state.user_id, None)
            .await
        {
            tracing::warn!("Failed to persist new thread: {}", e);
        }
        let metadata_val = json!("thread");
        if let Err(e) = store
            .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
            .await
        {
            tracing::warn!("Failed to set thread_type metadata: {}", e);
        }
        if let Some(matter_id) = active_matter
            && let Err(e) = store
                .bind_conversation_to_matter(thread_id, &state.user_id, &matter_id)
                .await
        {
            tracing::warn!(
                "Failed to bind new thread {} to matter {}: {}",
                thread_id,
                matter_id,
                e
            );
        }
    }

    Ok(Json(info))
}
