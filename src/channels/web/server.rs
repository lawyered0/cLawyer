//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    middleware,
    routing::{get, post},
};
use serde::Deserialize;
use tokio::sync::oneshot;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::channels::web::auth::{AuthState, auth_middleware};
use crate::channels::web::types::*;

#[cfg(test)]
use crate::agent::SessionManager;
#[cfg(test)]
use crate::channels::web::sse::SseManager;
pub use crate::channels::web::state::{GatewayState, PromptQueue, RateLimiter};
#[cfg(test)]
use axum::{
    Json,
    extract::{Multipart, Path, Query, State, WebSocketUpgrade},
    http::StatusCode,
    response::IntoResponse,
};

const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' https://cdn.jsdelivr.net; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self' https: wss:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'";

/// Start the gateway HTTP server.
///
/// Returns the actual bound `SocketAddr` (useful when binding to port 0).
pub async fn start_server(
    addr: SocketAddr,
    state: Arc<GatewayState>,
    auth_token: String,
) -> Result<SocketAddr, crate::error::ChannelError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!("Failed to bind to {}: {}", addr, e),
        }
    })?;
    let bound_addr =
        listener
            .local_addr()
            .map_err(|e| crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!("Failed to get local addr: {}", e),
            })?;

    // Public routes (no auth)
    let public = crate::channels::web::handlers::routes::public_routes();

    // Protected routes (require auth)
    let auth_state = AuthState { token: auth_token };
    let protected = Router::new()
        .merge(crate::channels::web::handlers::routes::protected_feature_routes())
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(crate::channels::web::openai_compat::chat_completions_handler),
        )
        .route(
            "/v1/models",
            get(crate::channels::web::openai_compat::models_handler),
        )
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

    // Static file routes (no auth, served from embedded strings)
    let statics = crate::channels::web::handlers::routes::static_routes();

    // Project file serving (behind auth to prevent unauthorized file access).
    let projects = crate::channels::web::handlers::routes::project_routes(auth_state.clone());

    // CORS: restrict to same-origin by default. Only localhost/127.0.0.1
    // origins are allowed, since the gateway is a local-first service.
    let gateway_origin_raw = format!("http://{}:{}", addr.ip(), addr.port());
    let gateway_origin = gateway_origin_raw.parse::<HeaderValue>().map_err(|err| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!(
                "Failed to build CORS origin header '{}': {}",
                gateway_origin_raw, err
            ),
        }
    })?;
    let localhost_origin_raw = format!("http://localhost:{}", addr.port());
    let localhost_origin = localhost_origin_raw.parse::<HeaderValue>().map_err(|err| {
        crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!(
                "Failed to build CORS origin header '{}': {}",
                localhost_origin_raw, err
            ),
        }
    })?;
    let cors = CorsLayer::new()
        .allow_origin([gateway_origin, localhost_origin])
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ])
        .allow_headers(AllowHeaders::list([
            header::CONTENT_TYPE,
            header::AUTHORIZATION,
        ]))
        .allow_credentials(true);

    let app = Router::new()
        .merge(public)
        .merge(statics)
        .merge(projects)
        .merge(protected)
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MB max request body
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(CONTENT_SECURITY_POLICY),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            header::HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            header::HeaderValue::from_static("DENY"),
        ))
        .with_state(state.clone());

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    *state.shutdown_tx.write().await = Some(shutdown_tx);

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
                tracing::info!("Web gateway shutting down");
            })
            .await
        {
            tracing::error!("Web gateway server error: {}", e);
        }
    });

    Ok(bound_addr)
}

// --- Static file handlers ---

// --- Chat handlers ---

pub(crate) async fn load_active_matter_for_chat(state: &GatewayState) -> Option<String> {
    let store = state.store.as_ref()?;
    let value = match store
        .get_setting(&state.user_id, MATTER_ACTIVE_SETTING)
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(
                "Failed to load active matter setting for chat metadata: {}",
                err
            );
            return None;
        }
    }?;
    let raw = value.as_str()?;
    crate::legal::policy::sanitize_optional_matter_id(raw)
}

pub(crate) async fn build_chat_message_metadata(
    state: &GatewayState,
    thread_id: Option<&str>,
) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    if let Some(thread_id) = thread_id {
        metadata.insert(
            "thread_id".to_string(),
            serde_json::Value::String(thread_id.to_string()),
        );
    }
    metadata.insert(
        "active_matter".to_string(),
        load_active_matter_for_chat(state)
            .await
            .map(serde_json::Value::String)
            .unwrap_or(serde_json::Value::Null),
    );
    serde_json::Value::Object(metadata)
}

#[cfg(test)]
async fn chat_send_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SendMessageRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_send_handler(State(state), Json(req)).await
}

#[cfg(test)]
async fn chat_approval_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<SendMessageResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_approval_handler(State(state), Json(req)).await
}

/// Submit an auth token directly to the extension manager, bypassing the message pipeline.
///
/// The token never touches the LLM, chat history, or SSE stream.
#[cfg(test)]
#[allow(dead_code)]
async fn chat_auth_token_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<AuthTokenRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_auth_token_handler(State(state), Json(req)).await
}

/// Cancel an in-progress auth flow.
#[cfg(test)]
#[allow(dead_code)]
async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_auth_cancel_handler(State(state), Json(req)).await
}

/// Clear pending auth mode on the active thread.
pub async fn clear_auth_mode(state: &GatewayState) {
    if let Some(ref sm) = state.session_manager {
        let session = sm.get_or_create_session(&state.user_id).await;
        let mut sess = session.lock().await;
        if let Some(thread_id) = sess.active_thread
            && let Some(thread) = sess.threads.get_mut(&thread_id)
        {
            thread.pending_auth = None;
        }
    }
}

#[cfg(test)]
#[allow(dead_code)]
async fn chat_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_events_handler(State(state)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_ws_handler(headers, ws, State(state)).await
}

#[derive(Deserialize)]
pub(crate) struct HistoryQuery {
    pub(crate) thread_id: Option<String>,
    pub(crate) limit: Option<usize>,
    pub(crate) before: Option<String>,
}

pub(crate) const CHAT_HISTORY_DEFAULT_LIMIT: usize = 50;
pub(crate) const CHAT_HISTORY_MIN_LIMIT: usize = 1;
pub(crate) const CHAT_HISTORY_MAX_LIMIT: usize = 200;

pub(crate) fn build_turns_from_session_thread(
    thread: &crate::agent::session::Thread,
) -> Vec<TurnInfo> {
    thread
        .turns
        .iter()
        .map(|t| TurnInfo {
            turn_number: t.turn_number,
            user_input: t.user_input.clone(),
            response: t.response.clone(),
            state: format!("{:?}", t.state),
            started_at: t.started_at.to_rfc3339(),
            completed_at: t.completed_at.map(|dt| dt.to_rfc3339()),
            tool_calls: t
                .tool_calls
                .iter()
                .map(|tc| ToolCallInfo {
                    name: tc.name.clone(),
                    has_result: tc.result.is_some(),
                    has_error: tc.error.is_some(),
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_history_handler(State(state), Query(query)).await
}

/// Build TurnInfo pairs from flat DB messages (alternating user/assistant).
pub(crate) fn build_turns_from_db_messages(
    messages: &[crate::history::ConversationMessage],
) -> Vec<TurnInfo> {
    let mut turns = Vec::new();
    let mut turn_number = 0;
    let mut iter = messages.iter().peekable();

    while let Some(msg) = iter.next() {
        if msg.role == "user" {
            let mut turn = TurnInfo {
                turn_number,
                user_input: msg.content.clone(),
                response: None,
                state: "Completed".to_string(),
                started_at: msg.created_at.to_rfc3339(),
                completed_at: None,
                tool_calls: Vec::new(),
            };

            // Check if next message is an assistant response
            if let Some(next) = iter.peek()
                && next.role == "assistant"
                && let Some(assistant_msg) = iter.next()
            {
                turn.response = Some(assistant_msg.content.clone());
                turn.completed_at = Some(assistant_msg.created_at.to_rfc3339());
            }

            // Incomplete turn (user message without response)
            if turn.response.is_none() {
                turn.state = "Failed".to_string();
            }

            turns.push(turn);
            turn_number += 1;
        }
    }

    turns
}

#[derive(Debug, Deserialize)]
pub(crate) struct ThreadListQuery {
    pub(crate) matter_id: Option<String>,
}

#[cfg(test)]
async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ThreadListQuery>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_threads_handler(State(state), Query(query)).await
}

#[cfg(test)]
async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::chat::chat_new_thread_handler(State(state)).await
}

// --- Memory handlers ---

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    #[allow(dead_code)]
    pub(crate) depth: Option<usize>,
}

#[cfg(test)]
#[allow(dead_code)]
async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_tree_handler(State(state), Query(query)).await
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    pub(crate) path: Option<String>,
}

#[cfg(test)]
#[allow(dead_code)]
async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_list_handler(State(state), Query(query)).await
}

#[derive(Deserialize)]
pub(crate) struct ReadQuery {
    pub(crate) path: String,
}

#[cfg(test)]
#[allow(dead_code)]
async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_read_handler(State(state), Query(query)).await
}

#[cfg(test)]
async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_write_handler(State(state), Json(req)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_search_handler(State(state), Json(req)).await
}

/// Maximum size accepted for a single uploaded file (10 MiB).
pub(crate) const UPLOAD_FILE_SIZE_LIMIT: usize = 10 * 1024 * 1024;
/// Maximum size accepted for backup restore uploads (128 MiB).
pub(crate) const BACKUP_RESTORE_SIZE_LIMIT: usize = 128 * 1024 * 1024;

#[cfg(test)]
#[allow(dead_code)]
async fn memory_upload_handler(
    State(state): State<Arc<GatewayState>>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<MemoryUploadResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::memory::memory_upload_handler(State(state), multipart).await
}

// --- Matter handlers ---

/// Default workspace path prefix where matter directories live.
pub(crate) use crate::channels::web::handlers::common::*;

#[cfg(test)]
#[allow(dead_code)]
async fn matters_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MattersListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matters_list_handler(State(state)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn clients_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ClientsQuery>,
) -> Result<Json<ClientsListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::clients_list_handler(
        State(state),
        Query(crate::channels::web::handlers::matters::core::ClientsQuery { q: query.q }),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn clients_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateClientRequest>,
) -> Result<(StatusCode, Json<ClientInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::clients_create_handler(State(state), Json(req))
        .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn clients_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::clients_get_handler(State(state), Path(id)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn clients_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateClientRequest>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::clients_patch_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn clients_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::clients_delete_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_get_handler(State(state), Path(id)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMatterRequest>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_patch_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_delete_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
async fn matter_tasks_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTasksListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_tasks_list_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
async fn matter_tasks_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterTaskRequest>,
) -> Result<(StatusCode, Json<MatterTaskInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_tasks_create_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_tasks_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, task_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterTaskRequest>,
) -> Result<Json<MatterTaskInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_tasks_patch_handler(
        State(state),
        Path((id, task_id)),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_tasks_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, task_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_tasks_delete_handler(
        State(state),
        Path((id, task_id)),
    )
    .await
}

#[cfg(test)]
async fn matter_notes_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterNotesListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_notes_list_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
async fn matter_notes_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterNoteRequest>,
) -> Result<(StatusCode, Json<MatterNoteInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_notes_create_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_notes_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, note_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterNoteRequest>,
) -> Result<Json<MatterNoteInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_notes_patch_handler(
        State(state),
        Path((id, note_id)),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_notes_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, note_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::work::matter_notes_delete_handler(
        State(state),
        Path((id, note_id)),
    )
    .await
}

#[cfg(test)]
async fn matter_time_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTimeEntriesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_time_list_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matter_time_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateTimeEntryRequest>,
) -> Result<(StatusCode, Json<TimeEntryInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_time_create_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_time_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, entry_id)): Path<(String, String)>,
    Json(req): Json<UpdateTimeEntryRequest>,
) -> Result<Json<TimeEntryInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_time_patch_handler(
        State(state),
        Path((id, entry_id)),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_time_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_time_delete_handler(
        State(state),
        Path((id, entry_id)),
    )
    .await
}

#[cfg(test)]
async fn matter_expenses_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterExpenseEntriesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_expenses_list_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matter_expenses_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateExpenseEntryRequest>,
) -> Result<(StatusCode, Json<ExpenseEntryInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_expenses_create_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_expenses_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, entry_id)): Path<(String, String)>,
    Json(req): Json<UpdateExpenseEntryRequest>,
) -> Result<Json<ExpenseEntryInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_expenses_patch_handler(
        State(state),
        Path((id, entry_id)),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_expenses_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_expenses_delete_handler(
        State(state),
        Path((id, entry_id)),
    )
    .await
}

#[cfg(test)]
async fn matter_time_summary_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTimeSummaryResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_time_summary_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matter_invoices_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<MatterInvoicesQuery>,
) -> Result<Json<MatterInvoicesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_invoices_list_handler(
        State(state),
        Path(id),
        Query(query),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn invoices_draft_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<DraftInvoiceRequest>,
) -> Result<Json<InvoiceDraftResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_draft_handler(
        State(state),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn invoices_save_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<DraftInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceDetailResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_save_handler(State(state), Json(req))
        .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn invoices_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_get_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
async fn invoices_finalize_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_finalize_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn invoices_void_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<InvoiceDetailResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_void_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
async fn invoices_payment_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<RecordInvoicePaymentRequest>,
) -> Result<Json<RecordInvoicePaymentResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::invoices_payment_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matter_trust_deposit_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<TrustDepositRequest>,
) -> Result<(StatusCode, Json<TrustLedgerEntryInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_trust_deposit_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matter_trust_ledger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<TrustLedgerResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::finance::matter_trust_ledger_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matters_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateMatterRequest>,
) -> Result<(StatusCode, Json<CreateMatterResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matters_create_handler(State(state), Json(req))
        .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matters_active_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ActiveMatterResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matters_active_get_handler(State(state)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matters_active_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SetActiveMatterRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matters_active_set_handler(
        State(state),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matter_documents_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<MatterDocumentsQuery>,
) -> Result<Json<MatterDocumentsResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_documents_handler(
        State(state),
        Path(id),
        Query(query),
    )
    .await
}

#[cfg(test)]
async fn matter_dashboard_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDashboardResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_dashboard_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_deadlines_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDeadlinesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_deadlines_handler(State(state), Path(id))
        .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_deadlines_create_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<CreateMatterDeadlineRequest>,
) -> Result<(StatusCode, Json<MatterDeadlineRecordInfo>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_deadlines_create_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_deadlines_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, deadline_id)): Path<(String, String)>,
    Json(req): Json<UpdateMatterDeadlineRequest>,
) -> Result<Json<MatterDeadlineRecordInfo>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_deadlines_patch_handler(
        State(state),
        Path((id, deadline_id)),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_deadlines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path((id, deadline_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_deadlines_delete_handler(
        State(state),
        Path((id, deadline_id)),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_deadlines_compute_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<MatterDeadlineComputeRequest>,
) -> Result<Json<MatterDeadlineComputeResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::core::matter_deadlines_compute_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn legal_court_rules_handler() -> Result<Json<CourtRulesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::legal::legal_court_rules_handler().await
}

#[cfg(test)]
async fn matter_templates_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTemplatesResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_templates_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matter_template_apply_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<MatterTemplateApplyRequest>,
) -> Result<(StatusCode, Json<MatterTemplateApplyResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_template_apply_handler(
        State(state),
        Path(id),
        Json(req),
    )
    .await
}

#[cfg(test)]
#[allow(dead_code)]
async fn matter_retrieval_export_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<MatterRetrievalExportRequest>>,
) -> Result<(StatusCode, Json<MatterRetrievalExportResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_retrieval_export_handler(
        State(state),
        Path(id),
        body,
    )
    .await
}

#[cfg(test)]
async fn documents_generate_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<GenerateDocumentRequest>,
) -> Result<(StatusCode, Json<GenerateDocumentResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::documents_generate_handler(
        State(state),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matter_filing_package_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<MatterFilingPackageResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::matters::documents::matter_filing_package_handler(
        State(state),
        Path(id),
    )
    .await
}

#[cfg(test)]
async fn matters_conflict_check_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MatterIntakeConflictCheckRequest>,
) -> Result<Json<MatterIntakeConflictCheckResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::conflicts::matters_conflict_check_handler(
        State(state),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matters_conflicts_check_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MatterConflictCheckRequest>,
) -> Result<Json<MatterConflictCheckResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::conflicts::matters_conflicts_check_handler(
        State(state),
        Json(req),
    )
    .await
}

#[cfg(test)]
async fn matters_conflicts_reindex_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MatterConflictGraphReindexResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::matters::conflicts::matters_conflicts_reindex_handler(State(
        state,
    ))
    .await
}

#[cfg(test)]
async fn legal_audit_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<LegalAuditQuery>,
) -> Result<Json<LegalAuditListResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::legal::legal_audit_list_handler(State(state), Query(query))
        .await
}

#[cfg(test)]
async fn compliance_status_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ComplianceStatusResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::legal::compliance_status_handler(State(state)).await
}

#[cfg(test)]
async fn compliance_letter_handler(
    State(state): State<Arc<GatewayState>>,
    body: Option<Json<ComplianceLetterRequest>>,
) -> Result<Json<ComplianceLetterResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::legal::compliance_letter_handler(State(state), body).await
}

// --- Jobs handlers ---

#[cfg(test)]
#[allow(dead_code)]
async fn backups_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<BackupCreateRequest>,
) -> Result<(StatusCode, Json<BackupCreateResponse>), (StatusCode, String)> {
    crate::channels::web::handlers::backups::backups_create_handler(State(state), Json(req)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn backups_verify_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<BackupVerifyRequest>,
) -> Result<Json<BackupVerifyResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::backups::backups_verify_handler(State(state), Json(req)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn backups_download_handler(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    crate::channels::web::handlers::backups::backups_download_handler(Path(id)).await
}

#[cfg(test)]
#[allow(dead_code)]
async fn backups_restore_handler(
    State(state): State<Arc<GatewayState>>,
    multipart: Multipart,
) -> Result<Json<BackupRestoreResponse>, (StatusCode, String)> {
    crate::channels::web::handlers::backups::backups_restore_handler(State(state), multipart).await
}

// --- Gateway control plane handlers ---

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
