//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::net::SocketAddr;
use std::path::{Component as FsComponent, Path as FsPath};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    middleware,
    routing::{get, post},
};
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use rust_decimal::Decimal;
use serde::Deserialize;
use tokio::sync::oneshot;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::channels::web::auth::{AuthState, auth_middleware};
use crate::channels::web::types::*;
use crate::db::{
    AuditSeverity, ClientType, ConflictClearanceRecord, ConflictDecision, ConflictHit,
    CreateClientParams, CreateDocumentVersionParams, CreateMatterDeadlineParams, ExpenseCategory,
    InvoiceLineItemRecord, InvoiceRecord, MatterDeadlineType, MatterDocumentCategory, MatterStatus,
    MatterTaskStatus, TrustLedgerEntryRecord, UpdateClientParams, UpdateMatterDeadlineParams,
    UpdateMatterParams, UpsertDocumentTemplateParams, UpsertMatterDocumentParams,
    UpsertMatterParams,
};
#[cfg(test)]
use crate::llm::CompletionRequest;
use crate::workspace::{Workspace, paths};

#[cfg(test)]
use crate::agent::SessionManager;
#[cfg(test)]
use crate::channels::web::sse::SseManager;
pub use crate::channels::web::state::{GatewayState, PromptQueue, RateLimiter};
#[cfg(test)]
use axum::{
    extract::{Multipart, WebSocketUpgrade},
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
        // Matters
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
        // OpenAI-compatible API
        .route(
            "/v1/chat/completions",
            post(super::openai_compat::chat_completions_handler),
        )
        .route("/v1/models", get(super::openai_compat::models_handler))
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
    let cors = CorsLayer::new()
        .allow_origin([
            format!("http://{}:{}", addr.ip(), addr.port())
                .parse()
                .expect("valid origin"),
            format!("http://localhost:{}", addr.port())
                .parse()
                .expect("valid origin"),
        ])
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
            {
                let assistant_msg = iter.next().expect("peeked");
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
const MATTER_ROOT: &str = "matters";
/// Settings key used to persist the active matter ID.
const MATTER_ACTIVE_SETTING: &str = "legal.active_matter";
/// Maximum number of party names accepted by intake conflict endpoints.
pub(crate) const MAX_INTAKE_CONFLICT_PARTIES: usize = 64;
/// Maximum length for a single intake party name.
const MAX_INTAKE_CONFLICT_PARTY_CHARS: usize = 160;
/// Maximum reminder offsets accepted for a single deadline.
const MAX_DEADLINE_REMINDERS: usize = 16;
/// Maximum allowed reminder offset in days.
const MAX_DEADLINE_REMINDER_DAYS: i32 = 3650;
/// Maximum allowed body text length for `/api/matters/conflicts/check`.
pub(crate) const MAX_CONFLICT_CHECK_TEXT_LEN: usize = 32 * 1024;
/// Default invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_DEFAULT_LIMIT: usize = 25;
/// Maximum invoice rows returned by `/api/matters/{id}/invoices`.
pub(crate) const MATTER_INVOICES_MAX_LIMIT: usize = 100;

/// Identity files that must not be overwritten through web memory-write APIs.
const PROTECTED_IDENTITY_FILES: &[&str] =
    &[paths::IDENTITY, paths::SOUL, paths::AGENTS, paths::USER];

pub(crate) fn legal_config_for_gateway(state: &GatewayState) -> crate::config::LegalConfig {
    state.legal_config.clone().unwrap_or_else(|| {
        crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve")
    })
}

pub(crate) fn matter_root_for_gateway(state: &GatewayState) -> String {
    let configured = legal_config_for_gateway(state).matter_root;
    let normalized = configured.trim_matches('/');
    if normalized.is_empty() {
        MATTER_ROOT.to_string()
    } else {
        normalized.to_string()
    }
}

fn matter_prefix_for_gateway(state: &GatewayState, matter_id: &str) -> String {
    format!("{}/{matter_id}", matter_root_for_gateway(state))
}

fn matter_metadata_path_for_gateway(state: &GatewayState, matter_id: &str) -> String {
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

    let legal = legal_config_for_gateway(state);
    let resolved_path = if legal.enabled && legal.require_matter_context {
        let matter_id = load_active_matter_for_chat(state)
            .await
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

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MatterDocumentsQuery {
    pub(crate) include_templates: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct MatterInvoicesQuery {
    pub(crate) limit: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
struct ClientsQuery {
    q: Option<String>,
}

pub(crate) fn sanitize_matter_id_for_route(raw: &str) -> Result<String, (StatusCode, String)> {
    let sanitized = crate::legal::policy::sanitize_matter_id(raw);
    if sanitized.is_empty() {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    Ok(sanitized)
}

pub(crate) async fn ensure_existing_matter_for_route(
    workspace: &Workspace,
    matter_root: &str,
    raw_matter_id: &str,
) -> Result<String, (StatusCode, String)> {
    let matter_id = sanitize_matter_id_for_route(raw_matter_id)?;
    match crate::legal::matter::read_matter_metadata_for_root(workspace, matter_root, &matter_id)
        .await
    {
        Ok(_) => Ok(matter_id),
        Err(crate::legal::matter::MatterMetadataValidationError::Missing { path }) => Err((
            StatusCode::NOT_FOUND,
            format!("Matter '{}' not found (missing '{}')", matter_id, path),
        )),
        Err(crate::legal::matter::MatterMetadataValidationError::Invalid { .. }) => Err((
            StatusCode::NOT_FOUND,
            format!("Matter '{}' metadata is invalid", matter_id),
        )),
        Err(err @ crate::legal::matter::MatterMetadataValidationError::Storage { .. }) => {
            Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
        }
    }
}

pub(crate) fn parse_template_name(raw: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must not be empty".to_string(),
        ));
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must be a basename under templates/".to_string(),
        ));
    }
    let path = FsPath::new(trimmed);
    let basename = path.file_name().and_then(|value| value.to_str()).ok_or((
        StatusCode::BAD_REQUEST,
        "'template_name' must be valid UTF-8".to_string(),
    ))?;
    if basename != trimmed {
        return Err((
            StatusCode::BAD_REQUEST,
            "'template_name' must be a basename under templates/".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) async fn choose_template_apply_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    template_name: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    let template_path = FsPath::new(template_name);
    let stem = template_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "'template_name' must include a valid file stem".to_string(),
        ))?;
    let ext = template_path.extension().and_then(|value| value.to_str());

    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let file_name = match ext {
            Some(ext) if !ext.is_empty() => format!("{stem}-{timestamp}{suffix}.{ext}"),
            _ => format!("{stem}-{timestamp}{suffix}"),
        };
        let candidate = format!("{matter_prefix}/drafts/{file_name}");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to pick a unique destination for applied template".to_string(),
    ))
}

pub(crate) async fn choose_generated_document_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    template_name: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    let parsed = FsPath::new(template_name);
    let stem = parsed
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("generated-document");
    let ext = parsed
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("md");

    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let candidate = format!("{matter_prefix}/drafts/{stem}-{timestamp}{suffix}.{ext}");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to pick a unique destination for generated document".to_string(),
    ))
}

pub(crate) async fn list_matter_documents_recursive(
    workspace: &Workspace,
    matter_prefix: &str,
    include_templates: bool,
) -> Result<Vec<MatterDocumentInfo>, (StatusCode, String)> {
    let mut pending = vec![matter_prefix.to_string()];
    let mut documents = Vec::new();
    let templates_prefix = format!("{matter_prefix}/templates");

    while let Some(path) = pending.pop() {
        let entries = workspace
            .list(&path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

        for entry in entries {
            if !include_templates
                && (entry.path == templates_prefix
                    || entry.path.starts_with(&format!("{templates_prefix}/")))
            {
                continue;
            }

            let name = entry.path.rsplit('/').next().unwrap_or("").to_string();
            if name.is_empty() {
                continue;
            }

            documents.push(MatterDocumentInfo {
                id: None,
                memory_document_id: None,
                name,
                display_name: None,
                path: entry.path.clone(),
                is_dir: entry.is_directory,
                category: None,
                updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
            });

            if entry.is_directory {
                pending.push(entry.path);
            }
        }
    }

    documents.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(documents)
}

pub(crate) fn checklist_completion_from_markdown(markdown: &str) -> (usize, usize) {
    let mut completed = 0usize;
    let mut total = 0usize;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = if let Some(rest) = trimmed.strip_prefix("- [") {
            rest
        } else {
            continue;
        };
        let mut chars = marker.chars();
        let state = chars.next().unwrap_or(' ');
        let bracket = chars.next().unwrap_or(' ');
        if bracket != ']' {
            continue;
        }
        total += 1;
        if matches!(state, 'x' | 'X' | '✓') {
            completed += 1;
        }
    }

    (completed, total)
}

fn parse_iso_date_token(input: &str) -> Option<(NaiveDate, usize, usize)> {
    let bytes = input.as_bytes();
    if bytes.len() < 10 {
        return None;
    }

    for start in 0..=bytes.len() - 10 {
        let token = &bytes[start..start + 10];
        let is_iso = token[0].is_ascii_digit()
            && token[1].is_ascii_digit()
            && token[2].is_ascii_digit()
            && token[3].is_ascii_digit()
            && token[4] == b'-'
            && token[5].is_ascii_digit()
            && token[6].is_ascii_digit()
            && token[7] == b'-'
            && token[8].is_ascii_digit()
            && token[9].is_ascii_digit();
        if !is_iso {
            continue;
        }

        let Ok(token_str) = std::str::from_utf8(token) else {
            continue;
        };
        let Ok(date) = NaiveDate::parse_from_str(token_str, "%Y-%m-%d") else {
            continue;
        };
        return Some((date, start, start + 10));
    }

    None
}

fn parse_deadlines_from_calendar(markdown: &str, today: NaiveDate) -> Vec<MatterDeadlineInfo> {
    let mut deadlines: Vec<(NaiveDate, MatterDeadlineInfo)> = Vec::new();

    for raw_line in markdown.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with("|---") {
            continue;
        }

        // Parse table rows first: | Date | Deadline / Event | Owner | Status | Source |
        if line.starts_with('|') {
            let normalized = line.trim_matches('|').trim();
            if normalized.is_empty() {
                continue;
            }
            let columns: Vec<&str> = line.split('|').map(str::trim).collect();
            // split('|') on a pipe-delimited row includes leading/trailing empty tokens.
            // We trim those by slicing, but keep interior empties to preserve column positions.
            let columns = if columns.len() >= 2 {
                &columns[1..columns.len() - 1]
            } else {
                &columns[..]
            };
            if columns.len() < 2
                || columns[0].eq_ignore_ascii_case("date")
                || columns[1].eq_ignore_ascii_case("deadline / event")
            {
                continue;
            }
            if let Some((date, _, _)) = parse_iso_date_token(columns[0]) {
                let title = columns.get(1).copied().unwrap_or("").trim().to_string();
                if title.is_empty() {
                    continue;
                }
                let owner = columns
                    .get(2)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let status = columns
                    .get(3)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());
                let source = columns
                    .get(4)
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .map(|value| value.to_string());

                deadlines.push((
                    date,
                    MatterDeadlineInfo {
                        date: date.to_string(),
                        title,
                        owner,
                        status,
                        source,
                        is_overdue: date < today,
                    },
                ));
                continue;
            }
        }

        // Fallback parser for checklist-style lines with embedded YYYY-MM-DD.
        if let Some((date, start, end)) = parse_iso_date_token(line) {
            let left = line[..start].trim();
            let right = line[end..].trim();
            let joined = if left.is_empty() {
                right.to_string()
            } else if right.is_empty() {
                left.to_string()
            } else {
                format!("{left} {right}")
            };
            let mut title = joined
                .trim()
                .trim_matches('|')
                .trim_matches('-')
                .trim()
                .to_string();
            if title.is_empty() {
                title = "Untitled deadline".to_string();
            }

            deadlines.push((
                date,
                MatterDeadlineInfo {
                    date: date.to_string(),
                    title,
                    owner: None,
                    status: None,
                    source: None,
                    is_overdue: date < today,
                },
            ));
        }
    }

    deadlines.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title)));
    deadlines.into_iter().map(|(_, info)| info).collect()
}

async fn read_matter_deadlines(
    workspace: &Workspace,
    matter_prefix: &str,
    today: NaiveDate,
) -> Result<Vec<MatterDeadlineInfo>, (StatusCode, String)> {
    let path = format!("{matter_prefix}/deadlines/calendar.md");
    match workspace.read(&path).await {
        Ok(doc) => Ok(parse_deadlines_from_calendar(&doc.content, today)),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(Vec::new()),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

pub(crate) async fn read_matter_deadlines_for_matter(
    state: &GatewayState,
    matter_id: &str,
    matter_prefix: &str,
    today: NaiveDate,
) -> Result<Vec<MatterDeadlineInfo>, (StatusCode, String)> {
    if let Some(store) = state.store.as_ref() {
        let records = store
            .list_matter_deadlines(&state.user_id, matter_id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        if !records.is_empty() {
            return Ok(records
                .iter()
                .map(deadline_record_to_legacy_info)
                .collect::<Vec<_>>());
        }
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    read_matter_deadlines(workspace.as_ref(), matter_prefix, today).await
}

pub(crate) async fn list_matter_templates(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Result<Vec<MatterTemplateInfo>, (StatusCode, String)> {
    let templates_path = format!("{matter_root}/{matter_id}/templates");
    let entries = match workspace.list(&templates_path).await {
        Ok(entries) => entries,
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Vec::new(),
        Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    };

    let mut templates: Vec<MatterTemplateInfo> = entries
        .into_iter()
        .filter(|entry| !entry.is_directory)
        .filter_map(|entry| {
            let name = entry.path.rsplit('/').next()?.to_string();
            if name.is_empty() {
                return None;
            }
            Some(MatterTemplateInfo {
                id: None,
                matter_id: Some(matter_id.to_string()),
                name,
                path: entry.path,
                variables_json: None,
                updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
            })
        })
        .collect();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

pub(crate) fn document_template_record_to_info(
    matter_root: &str,
    record: crate::db::DocumentTemplateRecord,
) -> MatterTemplateInfo {
    let path = match record.matter_id.as_ref() {
        Some(matter_id) => format!("{matter_root}/{matter_id}/templates/{}", record.name),
        None => format!("templates/shared/{}", record.name),
    };
    MatterTemplateInfo {
        id: Some(record.id.to_string()),
        matter_id: record.matter_id,
        name: record.name,
        path,
        variables_json: Some(record.variables_json),
        updated_at: Some(record.updated_at.to_rfc3339()),
    }
}

pub(crate) fn matter_document_record_to_info(
    record: crate::db::MatterDocumentRecord,
) -> MatterDocumentInfo {
    let fallback_name = record.path.rsplit('/').next().unwrap_or("").to_string();
    MatterDocumentInfo {
        id: Some(record.id.to_string()),
        memory_document_id: Some(record.memory_document_id.to_string()),
        name: fallback_name,
        display_name: Some(record.display_name),
        path: record.path,
        is_dir: false,
        category: Some(record.category.as_str().to_string()),
        updated_at: Some(record.updated_at.to_rfc3339()),
    }
}

pub(crate) async fn backfill_matter_templates_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let Some(workspace) = state.workspace.as_ref() else {
        return Ok(());
    };
    let matter_root = matter_root_for_gateway(state);
    let templates = list_matter_templates(workspace.as_ref(), &matter_root, matter_id).await?;
    for template in templates {
        let doc = workspace
            .read(&template.path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        store
            .upsert_document_template(
                &state.user_id,
                &UpsertDocumentTemplateParams {
                    matter_id: Some(matter_id.to_string()),
                    name: template.name,
                    body: doc.content,
                    variables_json: serde_json::json!([]),
                },
            )
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }
    Ok(())
}

pub(crate) async fn backfill_matter_documents_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let Some(workspace) = state.workspace.as_ref() else {
        return Ok(());
    };

    let matter_prefix = matter_prefix_for_gateway(state, matter_id);
    let docs = list_matter_documents_recursive(workspace.as_ref(), &matter_prefix, false).await?;
    for entry in docs.into_iter().filter(|item| !item.is_dir) {
        let doc = workspace
            .read(&entry.path)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let linked = store
            .upsert_matter_document(
                &state.user_id,
                matter_id,
                &UpsertMatterDocumentParams {
                    memory_document_id: doc.id,
                    path: doc.path.clone(),
                    display_name: entry.name.clone(),
                    category: infer_matter_document_category(&entry.path),
                },
            )
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let versions = store
            .list_document_versions(&state.user_id, linked.id)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        if versions.is_empty() {
            store
                .create_document_version(
                    &state.user_id,
                    &CreateDocumentVersionParams {
                        matter_document_id: linked.id,
                        label: "initial".to_string(),
                        memory_document_id: doc.id,
                    },
                )
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        }
    }

    Ok(())
}

pub(crate) async fn choose_filing_package_destination(
    workspace: &Workspace,
    matter_prefix: &str,
    timestamp: &str,
) -> Result<String, (StatusCode, String)> {
    for counter in 1usize..=999 {
        let suffix = if counter == 1 {
            String::new()
        } else {
            format!("-{}", counter)
        };
        let candidate = format!("{matter_prefix}/exports/filing-package-{timestamp}{suffix}.md");
        match workspace.read(&candidate).await {
            Ok(_) => continue,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => return Ok(candidate),
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    Err((
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to choose a unique filing package destination".to_string(),
    ))
}

pub(crate) async fn read_workspace_matter_metadata_optional(
    workspace: Option<&Arc<Workspace>>,
    matter_root: &str,
    matter_id: &str,
) -> Option<crate::legal::matter::MatterMetadata> {
    let workspace = workspace?;
    let path = format!("{matter_root}/{matter_id}/matter.yaml");
    let doc = workspace.read(&path).await.ok()?;
    serde_yml::from_str(&doc.content).ok()
}

async fn db_matter_to_info(state: &GatewayState, matter: crate::db::MatterRecord) -> MatterInfo {
    let matter_root = matter_root_for_gateway(state);
    let metadata = read_workspace_matter_metadata_optional(
        state.workspace.as_ref(),
        &matter_root,
        &matter.matter_id,
    )
    .await;
    let client_name = if let Some(store) = state.store.as_ref() {
        match store.get_client(&state.user_id, matter.client_id).await {
            Ok(Some(client)) => Some(client.name),
            _ => metadata.as_ref().map(|meta| meta.client.clone()),
        }
    } else {
        metadata.as_ref().map(|meta| meta.client.clone())
    };

    let opened_date = metadata
        .as_ref()
        .and_then(|meta| meta.opened_date.clone())
        .or_else(|| matter.opened_at.map(|dt| dt.date_naive().to_string()));

    MatterInfo {
        id: matter.matter_id.clone(),
        client_id: Some(matter.client_id.to_string()),
        client: client_name,
        status: Some(matter.status.as_str().to_string()),
        stage: matter.stage.clone(),
        confidentiality: metadata.as_ref().map(|meta| meta.confidentiality.clone()),
        team: if let Some(meta) = metadata.as_ref() {
            meta.team.clone()
        } else {
            matter.assigned_to.clone()
        },
        adversaries: metadata
            .as_ref()
            .map(|meta| meta.adversaries.clone())
            .unwrap_or_default(),
        retention: metadata.as_ref().map(|meta| meta.retention.clone()),
        jurisdiction: metadata
            .as_ref()
            .and_then(|meta| meta.jurisdiction.clone())
            .or(matter.jurisdiction.clone()),
        practice_area: metadata
            .as_ref()
            .and_then(|meta| meta.practice_area.clone())
            .or(matter.practice_area.clone()),
        opened_date: opened_date.clone(),
        opened_at: opened_date,
    }
}

pub(crate) async fn ensure_existing_matter_db(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let exists = store
        .get_matter_db(&state.user_id, matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some();
    if !exists {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    Ok(())
}

pub(crate) async fn ensure_matter_db_row_from_workspace(
    state: &GatewayState,
    matter_id: &str,
) -> Result<(), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    if store
        .get_matter_db(&state.user_id, matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_some()
    {
        return Ok(());
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state);
    let metadata = crate::legal::matter::read_matter_metadata_for_root(
        workspace.as_ref(),
        &matter_root,
        matter_id,
    )
    .await
    .map_err(|err| match err {
        crate::legal::matter::MatterMetadataValidationError::Missing { path } => (
            StatusCode::NOT_FOUND,
            format!("Matter '{}' not found (missing '{}')", matter_id, path),
        ),
        crate::legal::matter::MatterMetadataValidationError::Invalid { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, err.to_string())
        }
        crate::legal::matter::MatterMetadataValidationError::Storage { .. } => {
            (StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
        }
    })?;

    let client = store
        .upsert_client_by_normalized_name(
            &state.user_id,
            &CreateClientParams {
                name: metadata.client.clone(),
                client_type: ClientType::Entity,
                email: None,
                phone: None,
                address: None,
                notes: None,
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    let opened_at = parse_optional_datetime("opened_date", metadata.opened_date.clone())?;
    store
        .upsert_matter(
            &state.user_id,
            &UpsertMatterParams {
                matter_id: matter_id.to_string(),
                client_id: client.id,
                status: MatterStatus::Active,
                stage: None,
                practice_area: metadata.practice_area.clone(),
                jurisdiction: metadata.jurisdiction.clone(),
                opened_at,
                closed_at: None,
                assigned_to: metadata.team.clone(),
                custom_fields: serde_json::json!({}),
            },
        )
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok(())
}

async fn matters_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MattersListResponse>, (StatusCode, String)> {
    let matter_root = matter_root_for_gateway(state.as_ref());
    if let Some(store) = state.store.as_ref() {
        let matter_rows = store
            .list_matters_db(&state.user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let mut matters = Vec::with_capacity(matter_rows.len());
        for matter in matter_rows {
            matters.push(db_matter_to_info(state.as_ref(), matter).await);
        }
        matters.sort_by(|a, b| a.id.cmp(&b.id));
        return Ok(Json(MattersListResponse { matters }));
    }

    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let entries = list_matters_root_entries(workspace.list(&matter_root).await)?;
    let mut matters: Vec<MatterInfo> = Vec::new();
    for entry in entries.into_iter().filter(|entry| entry.is_directory) {
        let dir_name = entry.path.rsplit('/').next().unwrap_or("").to_string();
        if dir_name.is_empty() || dir_name == "_template" {
            continue;
        }
        let meta =
            read_workspace_matter_metadata_optional(Some(workspace), &matter_root, &dir_name).await;
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

async fn clients_list_handler(
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
        .map(client_record_to_info)
        .collect();
    Ok(Json(ClientsListResponse { clients }))
}

async fn clients_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateClientRequest>,
) -> Result<(StatusCode, Json<ClientInfo>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let name = parse_required_matter_field("name", &req.name)?;
    let client_type = parse_client_type(&req.client_type)?;
    let client = store
        .create_client(
            &state.user_id,
            &CreateClientParams {
                name,
                client_type,
                email: parse_optional_matter_field(req.email),
                phone: parse_optional_matter_field(req.phone),
                address: parse_optional_matter_field(req.address),
                notes: parse_optional_matter_field(req.notes),
            },
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(client_record_to_info(client))))
}

async fn clients_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = parse_uuid(&id, "id")?;
    let client = store
        .get_client(&state.user_id, client_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;
    Ok(Json(client_record_to_info(client)))
}

async fn clients_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateClientRequest>,
) -> Result<Json<ClientInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = parse_uuid(&id, "id")?;
    let input = UpdateClientParams {
        name: req.name.map(|value| value.trim().to_string()),
        client_type: req
            .client_type
            .as_deref()
            .map(parse_client_type)
            .transpose()?,
        email: req
            .email
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        phone: req
            .phone
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        address: req
            .address
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        notes: req
            .notes
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
    };

    let client = store
        .update_client(&state.user_id, client_id, &input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Client not found".to_string()))?;
    Ok(Json(client_record_to_info(client)))
}

async fn clients_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let client_id = parse_uuid(&id, "id")?;
    let deleted = store
        .delete_client(&state.user_id, client_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Client not found".to_string()));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn matter_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = sanitize_matter_id_for_route(&id)?;
    let matter = store
        .get_matter_db(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;
    Ok(Json(db_matter_to_info(state.as_ref(), matter).await))
}

async fn matter_patch_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<UpdateMatterRequest>,
) -> Result<Json<MatterInfo>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = sanitize_matter_id_for_route(&id)?;
    let client_id = req
        .client_id
        .as_deref()
        .map(|value| parse_uuid(value, "client_id"))
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
    let status = req.status.as_deref().map(parse_matter_status).transpose()?;

    let assigned_to = req.assigned_to.map(parse_matter_list);
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
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        practice_area: req
            .practice_area
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        jurisdiction: req
            .jurisdiction
            .map(|value| value.and_then(|inner| parse_optional_matter_field(Some(inner)))),
        opened_at: parse_optional_datetime_patch("opened_at", req.opened_at)?,
        closed_at: parse_optional_datetime_patch("closed_at", req.closed_at)?,
        assigned_to,
        custom_fields,
    };

    let matter = store
        .update_matter(&state.user_id, &matter_id, &input)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Matter not found".to_string()))?;

    if let Some(workspace) = state.workspace.as_ref() {
        let metadata_path = matter_metadata_path_for_gateway(state.as_ref(), &matter_id);
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
        record_legal_audit_event(
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

    Ok(Json(db_matter_to_info(state.as_ref(), matter).await))
}

async fn matter_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_id = sanitize_matter_id_for_route(&id)?;
    let deleted = store
        .delete_matter(&state.user_id, &matter_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    if let Some(active_value) = store
        .get_setting(&state.user_id, MATTER_ACTIVE_SETTING)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .and_then(|value| value.as_str().map(str::to_string))
        && crate::legal::policy::sanitize_matter_id(&active_value) == matter_id
    {
        store
            .delete_setting(&state.user_id, MATTER_ACTIVE_SETTING)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
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

pub(crate) fn parse_required_matter_field(
    field_name: &str,
    value: &str,
) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn parse_optional_matter_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_optional_matter_field_patch(value: Option<Option<String>>) -> Option<Option<String>> {
    match value {
        None => None,
        Some(None) => Some(None),
        Some(Some(raw)) => Some(parse_optional_matter_field(Some(raw))),
    }
}

const OPTIONAL_MATTER_FIELD_MAX_CHARS: usize = 256;

pub(crate) fn validate_optional_matter_field_length(
    field_name: &str,
    value: &Option<String>,
) -> Result<(), (StatusCode, String)> {
    if let Some(text) = value
        && text.chars().count() > OPTIONAL_MATTER_FIELD_MAX_CHARS
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' must be at most {} characters",
                field_name, OPTIONAL_MATTER_FIELD_MAX_CHARS
            ),
        ));
    }
    Ok(())
}

fn validate_opened_date(value: &str) -> Result<(), (StatusCode, String)> {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(parsed) if parsed.format("%Y-%m-%d").to_string() == value => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'opened_date' must be in YYYY-MM-DD format".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

fn parse_client_type(value: &str) -> Result<ClientType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "individual" => Ok(ClientType::Individual),
        "entity" => Ok(ClientType::Entity),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'client_type' must be 'individual' or 'entity'".to_string(),
        )),
    }
}

fn parse_matter_status(value: &str) -> Result<MatterStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "intake" => Ok(MatterStatus::Intake),
        "active" => Ok(MatterStatus::Active),
        "pending" => Ok(MatterStatus::Pending),
        "closed" => Ok(MatterStatus::Closed),
        "archived" => Ok(MatterStatus::Archived),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: intake, active, pending, closed, archived".to_string(),
        )),
    }
}

pub(crate) fn parse_matter_task_status(
    value: &str,
) -> Result<MatterTaskStatus, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "todo" => Ok(MatterTaskStatus::Todo),
        "in_progress" => Ok(MatterTaskStatus::InProgress),
        "done" => Ok(MatterTaskStatus::Done),
        "blocked" => Ok(MatterTaskStatus::Blocked),
        "cancelled" => Ok(MatterTaskStatus::Cancelled),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'status' must be one of: todo, in_progress, done, blocked, cancelled".to_string(),
        )),
    }
}

fn parse_matter_deadline_type(value: &str) -> Result<MatterDeadlineType, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "court_date" => Ok(MatterDeadlineType::CourtDate),
        "filing" => Ok(MatterDeadlineType::Filing),
        "statute_of_limitations" => Ok(MatterDeadlineType::StatuteOfLimitations),
        "response_due" => Ok(MatterDeadlineType::ResponseDue),
        "discovery_cutoff" => Ok(MatterDeadlineType::DiscoveryCutoff),
        "internal" => Ok(MatterDeadlineType::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'deadline_type' must be one of: court_date, filing, statute_of_limitations, response_due, discovery_cutoff, internal".to_string(),
        )),
    }
}

pub(crate) fn parse_expense_category(value: &str) -> Result<ExpenseCategory, (StatusCode, String)> {
    match value.trim().to_ascii_lowercase().as_str() {
        "filing_fee" => Ok(ExpenseCategory::FilingFee),
        "travel" => Ok(ExpenseCategory::Travel),
        "postage" => Ok(ExpenseCategory::Postage),
        "expert" => Ok(ExpenseCategory::Expert),
        "copying" => Ok(ExpenseCategory::Copying),
        "court_reporter" => Ok(ExpenseCategory::CourtReporter),
        "other" => Ok(ExpenseCategory::Other),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: filing_fee, travel, postage, expert, copying, court_reporter, other".to_string(),
        )),
    }
}

pub(crate) fn parse_date_only(
    field_name: &str,
    raw: &str,
) -> Result<NaiveDate, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let parsed = NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        )
    })?;
    if parsed.format("%Y-%m-%d").to_string() != value {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be in YYYY-MM-DD format", field_name),
        ));
    }
    Ok(parsed)
}

pub(crate) fn parse_decimal_field(
    field_name: &str,
    raw: &str,
) -> Result<Decimal, (StatusCode, String)> {
    let value = raw.trim();
    if value.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' is required", field_name),
        ));
    }
    let decimal = value.parse::<Decimal>().map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid decimal number", field_name),
        )
    })?;
    if decimal <= Decimal::ZERO {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' must be greater than 0", field_name),
        ));
    }
    Ok(decimal)
}

pub(crate) fn parse_optional_decimal_field(
    field_name: &str,
    raw: Option<String>,
) -> Result<Option<Decimal>, (StatusCode, String)> {
    match parse_optional_matter_field(raw) {
        Some(value) => parse_decimal_field(field_name, &value).map(Some),
        None => Ok(None),
    }
}

pub(crate) fn parse_matter_document_category(
    value: Option<&str>,
) -> Result<MatterDocumentCategory, (StatusCode, String)> {
    let raw = value.unwrap_or("internal").trim().to_ascii_lowercase();
    match raw.as_str() {
        "pleading" => Ok(MatterDocumentCategory::Pleading),
        "correspondence" => Ok(MatterDocumentCategory::Correspondence),
        "contract" => Ok(MatterDocumentCategory::Contract),
        "filing" => Ok(MatterDocumentCategory::Filing),
        "evidence" => Ok(MatterDocumentCategory::Evidence),
        "internal" | "" => Ok(MatterDocumentCategory::Internal),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'category' must be one of: pleading, correspondence, contract, filing, evidence, internal".to_string(),
        )),
    }
}

fn infer_matter_document_category(path: &str) -> MatterDocumentCategory {
    let lower = path.to_ascii_lowercase();
    if lower.contains("/filing") || lower.contains("/pleading") {
        MatterDocumentCategory::Filing
    } else if lower.contains("/evidence") {
        MatterDocumentCategory::Evidence
    } else if lower.contains("/contract") || lower.contains("/agreement") {
        MatterDocumentCategory::Contract
    } else if lower.contains("/correspondence") || lower.contains("/communication") {
        MatterDocumentCategory::Correspondence
    } else {
        MatterDocumentCategory::Internal
    }
}

fn normalize_reminder_days(values: &[i32]) -> Result<Vec<i32>, (StatusCode, String)> {
    use std::collections::BTreeSet;

    if values.len() > MAX_DEADLINE_REMINDERS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'reminder_days' supports at most {} values",
                MAX_DEADLINE_REMINDERS
            ),
        ));
    }

    let mut unique = BTreeSet::new();
    for day in values {
        if *day < 0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "'reminder_days' values must be >= 0".to_string(),
            ));
        }
        if *day > MAX_DEADLINE_REMINDER_DAYS {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "'reminder_days' values must be <= {}",
                    MAX_DEADLINE_REMINDER_DAYS
                ),
            ));
        }
        unique.insert(*day);
    }

    Ok(unique.into_iter().collect())
}

fn parse_datetime_value(field: &str, raw: &str) -> Result<DateTime<Utc>, (StatusCode, String)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("'{}' cannot be empty", field),
        ));
    }
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
        && let Some(dt) = date.and_hms_opt(0, 0, 0)
    {
        return Ok(dt.and_utc());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(dt.with_timezone(&Utc));
    }
    Err((
        StatusCode::BAD_REQUEST,
        format!("'{}' must be YYYY-MM-DD or RFC3339 datetime", field),
    ))
}

pub(crate) fn parse_optional_datetime(
    field: &str,
    raw: Option<String>,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Ok(None);
    }
    parse_datetime_value(field, &raw).map(Some)
}

pub(crate) fn parse_optional_datetime_patch(
    field: &str,
    raw: Option<Option<String>>,
) -> Result<Option<Option<DateTime<Utc>>>, (StatusCode, String)> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    if raw.trim().is_empty() {
        return Ok(Some(None));
    }
    Ok(Some(Some(parse_datetime_value(field, &raw)?)))
}

pub(crate) fn parse_uuid(value: &str, field: &str) -> Result<Uuid, (StatusCode, String)> {
    Uuid::parse_str(value).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            format!("'{}' must be a valid UUID", field),
        )
    })
}

fn parse_optional_uuid_field(
    value: Option<String>,
    field: &str,
) -> Result<Option<Uuid>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    parse_uuid(trimmed, field).map(Some)
}

fn parse_optional_uuid_patch_field(
    value: Option<Option<String>>,
    field: &str,
) -> Result<Option<Option<Uuid>>, (StatusCode, String)> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let Some(raw) = raw else {
        return Ok(Some(None));
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Some(None));
    }
    parse_uuid(trimmed, field).map(|uuid| Some(Some(uuid)))
}

fn deadline_record_to_info(record: crate::db::MatterDeadlineRecord) -> MatterDeadlineRecordInfo {
    let today = Utc::now().date_naive();
    let due_date = record.due_at.date_naive();
    MatterDeadlineRecordInfo {
        id: record.id.to_string(),
        title: record.title,
        deadline_type: record.deadline_type.as_str().to_string(),
        due_at: record.due_at.to_rfc3339(),
        completed_at: record.completed_at.map(|value| value.to_rfc3339()),
        reminder_days: record.reminder_days,
        rule_ref: record.rule_ref,
        computed_from: record.computed_from.map(|value| value.to_string()),
        task_id: record.task_id.map(|value| value.to_string()),
        is_overdue: record.completed_at.is_none() && due_date < today,
        days_until_due: due_date.signed_duration_since(today).num_days(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

fn deadline_record_to_legacy_info(record: &crate::db::MatterDeadlineRecord) -> MatterDeadlineInfo {
    let today = Utc::now().date_naive();
    let due_date = record.due_at.date_naive();
    let status = if record.completed_at.is_some() {
        Some("completed".to_string())
    } else {
        Some("open".to_string())
    };
    MatterDeadlineInfo {
        date: due_date.to_string(),
        title: record.title.clone(),
        owner: None,
        status,
        source: record.rule_ref.clone(),
        is_overdue: record.completed_at.is_none() && due_date < today,
    }
}

fn deadline_compute_preview_from_params(
    params: &CreateMatterDeadlineParams,
) -> MatterDeadlineComputePreview {
    let today = Utc::now().date_naive();
    let due_date = params.due_at.date_naive();
    MatterDeadlineComputePreview {
        title: params.title.clone(),
        deadline_type: params.deadline_type.as_str().to_string(),
        due_at: params.due_at.to_rfc3339(),
        reminder_days: params.reminder_days.clone(),
        rule_ref: params.rule_ref.clone(),
        computed_from: params.computed_from.map(|value| value.to_string()),
        task_id: params.task_id.map(|value| value.to_string()),
        is_overdue: due_date < today,
        days_until_due: due_date.signed_duration_since(today).num_days(),
    }
}

fn deadline_reminder_prefix(matter_id: &str, deadline_id: Uuid) -> String {
    format!("deadline-reminder-{matter_id}-{deadline_id}-")
}

fn deadline_reminder_name(matter_id: &str, deadline_id: Uuid, reminder_days: i32) -> String {
    format!(
        "{}{}",
        deadline_reminder_prefix(matter_id, deadline_id),
        reminder_days
    )
}

fn deadline_reminder_schedule(run_at: DateTime<Utc>) -> String {
    format!(
        "{} {} {} {} {} *",
        run_at.second(),
        run_at.minute(),
        run_at.hour(),
        run_at.day(),
        run_at.month()
    )
}

async fn disable_deadline_reminder_routines(
    state: &GatewayState,
    matter_id: &str,
    deadline_id: Uuid,
) -> Result<(), (StatusCode, String)> {
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let prefix = deadline_reminder_prefix(matter_id, deadline_id);
    let routines = store
        .list_routines(&state.user_id)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    for mut routine in routines {
        if !routine.name.starts_with(&prefix) || !routine.enabled {
            continue;
        }
        routine.enabled = false;
        routine.next_fire_at = None;
        store
            .update_routine(&routine)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }

    Ok(())
}

async fn sync_deadline_reminder_routines_for_record(
    state: &GatewayState,
    record: &crate::db::MatterDeadlineRecord,
) -> Result<(), (StatusCode, String)> {
    disable_deadline_reminder_routines(state, &record.matter_id, record.id).await?;
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };

    if record.completed_at.is_some() || record.reminder_days.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    for reminder_days in &record.reminder_days {
        let run_at = record.due_at - chrono::Duration::days(i64::from(*reminder_days));
        if run_at <= now {
            continue;
        }

        let name = deadline_reminder_name(&record.matter_id, record.id, *reminder_days);
        let schedule = deadline_reminder_schedule(run_at);
        let next_fire = crate::agent::routine::next_cron_fire(&schedule)
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        let prompt = format!(
            "Matter `{}` deadline reminder: \"{}\" is due on {} ({} days remaining). Provide a concise reminder and immediate next action.",
            record.matter_id,
            record.title,
            record.due_at.date_naive(),
            reminder_days
        );
        let state_json = serde_json::json!({
            "one_shot": true,
            "deadline_id": record.id,
            "matter_id": record.matter_id,
            "reminder_days": reminder_days,
        });

        if let Some(mut existing) = store
            .get_routine_by_name(&state.user_id, &name)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        {
            existing.enabled = true;
            existing.trigger = crate::agent::routine::Trigger::Cron { schedule };
            existing.action = crate::agent::routine::RoutineAction::Lightweight {
                prompt: prompt.clone(),
                context_paths: vec![matter_metadata_path_for_gateway(state, &record.matter_id)],
                max_tokens: 300,
            };
            existing.next_fire_at = next_fire;
            existing.state = state_json.clone();
            store
                .update_routine(&existing)
                .await
                .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
            continue;
        }

        let routine = crate::agent::routine::Routine {
            id: Uuid::new_v4(),
            name,
            description: format!(
                "One-shot reminder {} day(s) before deadline '{}'",
                reminder_days, record.title
            ),
            user_id: state.user_id.clone(),
            enabled: true,
            trigger: crate::agent::routine::Trigger::Cron { schedule },
            action: crate::agent::routine::RoutineAction::Lightweight {
                prompt,
                context_paths: vec![matter_metadata_path_for_gateway(state, &record.matter_id)],
                max_tokens: 300,
            },
            guardrails: crate::agent::routine::RoutineGuardrails::default(),
            notify: crate::agent::routine::NotifyConfig::default(),
            last_run_at: None,
            next_fire_at: next_fire,
            run_count: 0,
            consecutive_failures: 0,
            state: state_json,
            created_at: now,
            updated_at: now,
        };
        store
            .create_routine(&routine)
            .await
            .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    }

    Ok(())
}

pub(crate) fn parse_uuid_list(
    values: &[String],
    field: &str,
) -> Result<Vec<Uuid>, (StatusCode, String)> {
    values
        .iter()
        .map(|value| parse_uuid(value, field))
        .collect()
}

fn client_record_to_info(client: crate::db::ClientRecord) -> ClientInfo {
    ClientInfo {
        id: client.id.to_string(),
        name: client.name,
        client_type: client.client_type.as_str().to_string(),
        email: client.email,
        phone: client.phone,
        address: client.address,
        notes: client.notes,
        created_at: client.created_at.to_rfc3339(),
        updated_at: client.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_task_record_to_info(task: crate::db::MatterTaskRecord) -> MatterTaskInfo {
    MatterTaskInfo {
        id: task.id.to_string(),
        title: task.title,
        description: task.description,
        status: task.status.as_str().to_string(),
        assignee: task.assignee,
        due_at: task.due_at.map(|dt| dt.to_rfc3339()),
        blocked_by: task
            .blocked_by
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_note_record_to_info(note: crate::db::MatterNoteRecord) -> MatterNoteInfo {
    MatterNoteInfo {
        id: note.id.to_string(),
        author: note.author,
        body: note.body,
        pinned: note.pinned,
        created_at: note.created_at.to_rfc3339(),
        updated_at: note.updated_at.to_rfc3339(),
    }
}

pub(crate) fn time_entry_record_to_info(entry: crate::db::TimeEntryRecord) -> TimeEntryInfo {
    TimeEntryInfo {
        id: entry.id.to_string(),
        timekeeper: entry.timekeeper,
        description: entry.description,
        hours: entry.hours.to_string(),
        hourly_rate: entry.hourly_rate.map(|value| value.to_string()),
        entry_date: entry.entry_date.to_string(),
        billable: entry.billable,
        billed_invoice_id: entry.billed_invoice_id,
        created_at: entry.created_at.to_rfc3339(),
        updated_at: entry.updated_at.to_rfc3339(),
    }
}

pub(crate) fn expense_entry_record_to_info(
    entry: crate::db::ExpenseEntryRecord,
) -> ExpenseEntryInfo {
    ExpenseEntryInfo {
        id: entry.id.to_string(),
        submitted_by: entry.submitted_by,
        description: entry.description,
        amount: entry.amount.to_string(),
        category: entry.category.as_str().to_string(),
        entry_date: entry.entry_date.to_string(),
        receipt_path: entry.receipt_path,
        billable: entry.billable,
        billed_invoice_id: entry.billed_invoice_id,
        created_at: entry.created_at.to_rfc3339(),
        updated_at: entry.updated_at.to_rfc3339(),
    }
}

pub(crate) fn matter_time_summary_to_response(
    summary: crate::db::MatterTimeSummary,
) -> MatterTimeSummaryResponse {
    MatterTimeSummaryResponse {
        total_hours: summary.total_hours.to_string(),
        billable_hours: summary.billable_hours.to_string(),
        unbilled_hours: summary.unbilled_hours.to_string(),
        total_expenses: summary.total_expenses.to_string(),
        billable_expenses: summary.billable_expenses.to_string(),
        unbilled_expenses: summary.unbilled_expenses.to_string(),
    }
}

pub(crate) fn invoice_record_to_info(invoice: InvoiceRecord) -> InvoiceInfo {
    InvoiceInfo {
        id: invoice.id.to_string(),
        matter_id: invoice.matter_id,
        invoice_number: invoice.invoice_number,
        status: invoice.status.as_str().to_string(),
        issued_date: invoice.issued_date.map(|value| value.to_string()),
        due_date: invoice.due_date.map(|value| value.to_string()),
        subtotal: invoice.subtotal.to_string(),
        tax: invoice.tax.to_string(),
        total: invoice.total.to_string(),
        paid_amount: invoice.paid_amount.to_string(),
        notes: invoice.notes,
        created_at: invoice.created_at.to_rfc3339(),
        updated_at: invoice.updated_at.to_rfc3339(),
    }
}

pub(crate) fn invoice_draft_to_info(invoice: &crate::db::CreateInvoiceParams) -> InvoiceDraftInfo {
    InvoiceDraftInfo {
        matter_id: invoice.matter_id.clone(),
        invoice_number: invoice.invoice_number.clone(),
        status: invoice.status.as_str().to_string(),
        due_date: invoice.due_date.map(|value| value.to_string()),
        subtotal: invoice.subtotal.to_string(),
        tax: invoice.tax.to_string(),
        total: invoice.total.to_string(),
        notes: invoice.notes.clone(),
    }
}

pub(crate) fn invoice_line_item_record_to_info(item: InvoiceLineItemRecord) -> InvoiceLineItemInfo {
    InvoiceLineItemInfo {
        id: item.id.to_string(),
        description: item.description,
        quantity: item.quantity.to_string(),
        unit_price: item.unit_price.to_string(),
        amount: item.amount.to_string(),
        time_entry_id: item.time_entry_id.map(|value| value.to_string()),
        expense_entry_id: item.expense_entry_id.map(|value| value.to_string()),
        sort_order: item.sort_order,
    }
}

pub(crate) fn invoice_line_item_params_to_info(
    item: &crate::db::CreateInvoiceLineItemParams,
) -> InvoiceLineItemInfo {
    InvoiceLineItemInfo {
        id: "draft".to_string(),
        description: item.description.clone(),
        quantity: item.quantity.to_string(),
        unit_price: item.unit_price.to_string(),
        amount: item.amount.to_string(),
        time_entry_id: item.time_entry_id.map(|value| value.to_string()),
        expense_entry_id: item.expense_entry_id.map(|value| value.to_string()),
        sort_order: item.sort_order,
    }
}

pub(crate) fn trust_ledger_entry_record_to_info(
    entry: TrustLedgerEntryRecord,
) -> TrustLedgerEntryInfo {
    TrustLedgerEntryInfo {
        id: entry.id.to_string(),
        matter_id: entry.matter_id,
        entry_type: entry.entry_type.as_str().to_string(),
        amount: entry.amount.to_string(),
        balance_after: entry.balance_after.to_string(),
        description: entry.description,
        invoice_id: entry.invoice_id.map(|value| value.to_string()),
        recorded_by: entry.recorded_by,
        created_at: entry.created_at.to_rfc3339(),
    }
}

pub(crate) fn audit_event_record_to_info(
    event: crate::db::AuditEventRecord,
) -> LegalAuditEventInfo {
    LegalAuditEventInfo {
        id: event.id.to_string(),
        ts: event.created_at.to_rfc3339(),
        event_type: event.event_type,
        actor: event.actor,
        matter_id: event.matter_id,
        severity: event.severity.as_str().to_string(),
        details: event.details,
    }
}

fn validate_intake_party_name(field_name: &str, value: &str) -> Result<(), (StatusCode, String)> {
    if value.chars().count() > MAX_INTAKE_CONFLICT_PARTY_CHARS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' entries must be at most {} characters",
                field_name, MAX_INTAKE_CONFLICT_PARTY_CHARS
            ),
        ));
    }
    Ok(())
}

pub(crate) fn validate_intake_party_list(
    field_name: &str,
    values: &[String],
) -> Result<(), (StatusCode, String)> {
    if values.len() > MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "'{}' may include at most {} names",
                field_name, MAX_INTAKE_CONFLICT_PARTIES
            ),
        ));
    }
    for value in values {
        validate_intake_party_name(field_name, value)?;
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

pub(crate) fn list_matters_root_entries(
    result: Result<Vec<crate::workspace::WorkspaceEntry>, crate::error::WorkspaceError>,
) -> Result<Vec<crate::workspace::WorkspaceEntry>, (StatusCode, String)> {
    match result {
        Ok(entries) => Ok(entries),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => Ok(Vec::new()),
        Err(err) => Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

async fn matters_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<CreateMatterRequest>,
) -> Result<(StatusCode, Json<CreateMatterResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state.as_ref());

    let raw_matter_id = parse_required_matter_field("matter_id", &req.matter_id)?;
    let sanitized = crate::legal::policy::sanitize_matter_id(&raw_matter_id);
    if sanitized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Matter ID is empty after sanitization".to_string(),
        ));
    }

    let existing = list_matters_root_entries(workspace.list(&matter_root).await)?;
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

    let client = parse_required_matter_field("client", &req.client)?;
    let confidentiality = parse_required_matter_field("confidentiality", &req.confidentiality)?;
    let retention = parse_required_matter_field("retention", &req.retention)?;
    validate_intake_party_name("client", &client)?;
    let jurisdiction = parse_optional_matter_field(req.jurisdiction);
    let practice_area = parse_optional_matter_field(req.practice_area);
    let opened_date = parse_optional_matter_field(req.opened_date.or(req.opened_at));
    validate_optional_matter_field_length("jurisdiction", &jurisdiction)?;
    validate_optional_matter_field_length("practice_area", &practice_area)?;
    if let Some(value) = opened_date.as_deref() {
        validate_opened_date(value)?;
    }
    let team = parse_matter_list(req.team);
    let adversaries = parse_matter_list(req.adversaries);
    validate_intake_party_list("adversaries", &adversaries)?;
    let conflict_decision = req.conflict_decision;
    let conflict_note = parse_optional_matter_field(req.conflict_note);
    let checked_parties = build_checked_parties(&client, &adversaries);
    if checked_parties.len() > MAX_INTAKE_CONFLICT_PARTIES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "combined conflict-check parties may include at most {} names",
                MAX_INTAKE_CONFLICT_PARTIES
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

        if matches!(
            decision,
            ConflictDecision::Waived | ConflictDecision::Declined
        ) && conflict_note.as_deref().is_none()
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "'conflict_note' is required for waived or declined decisions".to_string(),
            ));
        }

        let clearance = ConflictClearanceRecord {
            matter_id: sanitized.clone(),
            checked_by: state.user_id.clone(),
            cleared_by: if matches!(decision, ConflictDecision::Declined) {
                None
            } else {
                Some(state.user_id.clone())
            },
            decision,
            note: conflict_note.clone(),
            hits_json: serde_json::to_value(&conflict_hits)
                .unwrap_or_else(|_| serde_json::json!([])),
            hit_count: conflict_hits.len() as i32,
        };
        store
            .record_conflict_clearance(&clearance)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        record_legal_audit_event(
            state.as_ref(),
            "conflict_clearance_decision",
            state.user_id.as_str(),
            Some(sanitized.as_str()),
            AuditSeverity::Info,
            serde_json::json!({
                "matter_id": sanitized.clone(),
                "decision": decision.as_str(),
                "checked_by": state.user_id.clone(),
                "cleared_by_present": clearance.cleared_by.is_some(),
                "hit_count": clearance.hit_count,
                "source": "create_flow",
            }),
        )
        .await;

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

    let opened_at_ts = parse_optional_datetime("opened_date", opened_date.clone())?;
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
        .set_setting(&state.user_id, MATTER_ACTIVE_SETTING, &value)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    record_legal_audit_event(
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

async fn matters_active_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ActiveMatterResponse>, (StatusCode, String)> {
    let matter_root = matter_root_for_gateway(state.as_ref());
    let mut matter_id = if let Some(store) = state.store.as_ref() {
        store
            .get_setting(&state.user_id, MATTER_ACTIVE_SETTING)
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

async fn matters_active_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<SetActiveMatterRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state.as_ref());

    let trimmed = req
        .matter_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match trimmed {
        None => {
            // Clear active matter.
            store
                .delete_setting(&state.user_id, MATTER_ACTIVE_SETTING)
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
                    MATTER_ACTIVE_SETTING,
                    &serde_json::Value::String(sanitized),
                )
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
    }

    Ok(StatusCode::NO_CONTENT)
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

async fn matter_deadlines_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDeadlinesResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state.as_ref());
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id).await?;
    let matter_prefix = format!("{matter_root}/{matter_id}");
    let deadlines = read_matter_deadlines_for_matter(
        state.as_ref(),
        &matter_id,
        &matter_prefix,
        Utc::now().date_naive(),
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

async fn matter_deadlines_create_handler(
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
    let matter_root = matter_root_for_gateway(state.as_ref());
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id).await?;
    ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id).await?;

    let title = req.title.trim();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "'title' is required".to_string()));
    }
    let deadline_type = parse_matter_deadline_type(&req.deadline_type)?;
    let due_at = parse_datetime_value("due_at", &req.due_at)?;
    let completed_at = parse_optional_datetime("completed_at", req.completed_at)?;
    let reminder_days = normalize_reminder_days(&req.reminder_days)?;
    let rule_ref = parse_optional_matter_field(req.rule_ref);
    validate_optional_matter_field_length("rule_ref", &rule_ref)?;
    let computed_from = parse_optional_uuid_field(req.computed_from, "computed_from")?;
    let task_id = parse_optional_uuid_field(req.task_id, "task_id")?;

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

    sync_deadline_reminder_routines_for_record(state.as_ref(), &created).await?;

    Ok((StatusCode::CREATED, Json(deadline_record_to_info(created))))
}

async fn matter_deadlines_patch_handler(
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
    let matter_root = matter_root_for_gateway(state.as_ref());
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id).await?;
    ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id).await?;
    let deadline_id = parse_uuid(deadline_id.trim(), "deadline_id")?;

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
        .map(parse_matter_deadline_type)
        .transpose()?;
    let due_at = req
        .due_at
        .as_deref()
        .map(|value| parse_datetime_value("due_at", value))
        .transpose()?;
    let completed_at = parse_optional_datetime_patch("completed_at", req.completed_at)?;
    let reminder_days = req
        .reminder_days
        .as_ref()
        .map(|values| normalize_reminder_days(values))
        .transpose()?;
    let rule_ref = parse_optional_matter_field_patch(req.rule_ref);
    if let Some(Some(ref value)) = rule_ref {
        validate_optional_matter_field_length("rule_ref", &Some(value.clone()))?;
    }
    let computed_from = parse_optional_uuid_patch_field(req.computed_from, "computed_from")?;
    let task_id = parse_optional_uuid_patch_field(req.task_id, "task_id")?;

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

    sync_deadline_reminder_routines_for_record(state.as_ref(), &updated).await?;

    Ok(Json(deadline_record_to_info(updated)))
}

async fn matter_deadlines_delete_handler(
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
    let matter_root = matter_root_for_gateway(state.as_ref());
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id).await?;
    ensure_matter_db_row_from_workspace(state.as_ref(), &matter_id).await?;
    let deadline_id = parse_uuid(deadline_id.trim(), "deadline_id")?;

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

    disable_deadline_reminder_routines(state.as_ref(), &existing.matter_id, existing.id).await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn matter_deadlines_compute_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<MatterDeadlineComputeRequest>,
) -> Result<Json<MatterDeadlineComputeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_root = matter_root_for_gateway(state.as_ref());
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &matter_root, &id).await?;
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
    let trigger = parse_datetime_value("trigger_date", &req.trigger_date)?;
    let reminder_days = normalize_reminder_days(&req.reminder_days)?;
    let computed_from = parse_optional_uuid_field(req.computed_from, "computed_from")?;
    let task_id = parse_optional_uuid_field(req.task_id, "task_id")?;
    let title = parse_optional_matter_field(req.title)
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
        deadline: deadline_compute_preview_from_params(&computed),
    }))
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
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use regex::Regex;

    struct TestLlmProvider {
        model: String,
        content: String,
    }

    #[async_trait]
    impl crate::llm::LlmProvider for TestLlmProvider {
        fn model_name(&self) -> &str {
            &self.model
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, crate::error::LlmError> {
            Ok(crate::llm::CompletionResponse {
                content: self.content.clone(),
                input_tokens: 12,
                output_tokens: 34,
                finish_reason: crate::llm::FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: crate::llm::ToolCompletionRequest,
        ) -> Result<crate::llm::ToolCompletionResponse, crate::error::LlmError> {
            Ok(crate::llm::ToolCompletionResponse {
                content: Some(self.content.clone()),
                tool_calls: Vec::new(),
                input_tokens: 12,
                output_tokens: 34,
                finish_reason: crate::llm::FinishReason::Stop,
            })
        }
    }

    fn minimal_test_gateway_state(
        llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            user_id: "test-user".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(
                crate::channels::web::ws::WsConnectionTracker::new(),
            )),
            llm_provider,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            startup_time: std::time::Instant::now(),
            legal_config: Some(
                crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
                    .expect("default legal config"),
            ),
            runtime_facts: crate::compliance::ComplianceRuntimeFacts::default(),
        })
    }

    fn assert_no_inline_event_handlers(asset_name: &str, content: &str) {
        let patterns = ["onclick=", "onchange=", "oninput="];
        for pattern in patterns {
            assert!(
                !content.contains(pattern),
                "{} unexpectedly contains inline event handler pattern '{}'",
                asset_name,
                pattern
            );
        }
    }

    #[test]
    fn test_build_turns_from_db_messages_complete() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi there!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "How are you?".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Doing well!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(3),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_input, "Hello");
        assert_eq!(turns[0].response.as_deref(), Some("Hi there!"));
        assert_eq!(turns[0].state, "Completed");
        assert_eq!(turns[1].user_input, "How are you?");
        assert_eq!(turns[1].response.as_deref(), Some("Doing well!"));
    }

    #[test]
    fn test_build_turns_from_db_messages_incomplete_last() {
        let now = chrono::Utc::now();
        let messages = vec![
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Hello".to_string(),
                created_at: now,
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "assistant".to_string(),
                content: "Hi!".to_string(),
                created_at: now + chrono::TimeDelta::seconds(1),
            },
            crate::history::ConversationMessage {
                id: Uuid::new_v4(),
                role: "user".to_string(),
                content: "Lost message".to_string(),
                created_at: now + chrono::TimeDelta::seconds(2),
            },
        ];

        let turns = build_turns_from_db_messages(&messages);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[1].user_input, "Lost message");
        assert!(turns[1].response.is_none());
        assert_eq!(turns[1].state, "Failed");
    }

    #[test]
    fn test_build_turns_from_db_messages_empty() {
        let turns = build_turns_from_db_messages(&[]);
        assert!(turns.is_empty());
    }

    #[test]
    fn test_index_html_has_no_inline_event_handlers() {
        let index = include_str!("static/index.html");
        assert_no_inline_event_handlers("index.html", index);
    }

    #[test]
    fn test_app_js_has_no_inline_event_handlers() {
        let app_js = include_str!("static/app.js");
        assert_no_inline_event_handlers("app.js", app_js);
    }

    #[test]
    fn test_app_js_contains_delegated_action_hooks() {
        let app_js = include_str!("static/app.js");
        let required_markers = [
            "data-job-action",
            "data-routine-action",
            "data-memory-nav-path",
            "data-tee-action=\"copy-report\"",
        ];
        for marker in required_markers {
            assert!(
                app_js.contains(marker),
                "app.js missing delegated action marker: {}",
                marker
            );
        }

        let delegate_calls = [
            r"delegate\(byId\('jobs-tbody'\),\s*'click',\s*'button\[data-job-action\]'",
            r"delegate\(byId\('routines-tbody'\),\s*'click',\s*'button\[data-routine-action\]'",
            r"delegate\(\s*byId\('memory-breadcrumb-path'\),\s*'click',\s*'a\[data-memory-nav-root\],a\[data-memory-nav-path\]'",
        ];
        for pattern in delegate_calls {
            let re = Regex::new(pattern).expect("valid delegate regex");
            assert!(
                re.is_match(app_js),
                "missing delegate call matching {}",
                pattern
            );
        }

        let refresh_calls = app_js.matches("refreshActiveMatterState();").count();
        assert!(
            refresh_calls >= 2,
            "expected at least two refreshActiveMatterState() call sites, found {}",
            refresh_calls
        );
    }

    #[test]
    fn test_index_html_contains_compliance_section_markers() {
        let index = include_str!("static/index.html");
        assert!(
            index.contains("settings-compliance-status"),
            "index.html is missing compliance status container marker"
        );
        assert!(
            index.contains("settings-compliance-letter-btn"),
            "index.html is missing compliance letter button marker"
        );
    }

    #[test]
    fn test_app_js_contains_compliance_api_calls() {
        let app_js = include_str!("static/app.js");
        assert!(
            app_js.contains("/api/compliance/status"),
            "app.js missing compliance status API call"
        );
        assert!(
            app_js.contains("/api/compliance/letter"),
            "app.js missing compliance letter API call"
        );
    }

    #[tokio::test]
    async fn compliance_status_handler_returns_ok_without_db() {
        let state = minimal_test_gateway_state(None);
        let Json(response) = compliance_status_handler(State(state))
            .await
            .expect("status response");
        assert_eq!(response.overall, ComplianceStatusLevel::NeedsReview);
        assert!(
            response
                .data_gaps
                .iter()
                .any(|gap| gap.to_ascii_lowercase().contains("database")),
            "expected data_gaps to include a database-unavailable note"
        );
    }

    #[tokio::test]
    async fn compliance_letter_handler_rejects_invalid_framework() {
        let state = minimal_test_gateway_state(None);
        let err = compliance_letter_handler(
            State(state),
            Some(Json(ComplianceLetterRequest {
                framework: Some("invalid-framework".to_string()),
                firm_name: None,
            })),
        )
        .await
        .expect_err("invalid framework should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn compliance_letter_handler_requires_llm_provider() {
        let state = minimal_test_gateway_state(None);
        let err = compliance_letter_handler(
            State(state),
            Some(Json(ComplianceLetterRequest {
                framework: Some("nist".to_string()),
                firm_name: Some("Example LLP".to_string()),
            })),
        )
        .await
        .expect_err("missing llm should fail");
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn compliance_letter_handler_appends_disclaimer() {
        let llm = Arc::new(TestLlmProvider {
            model: "test-model".to_string(),
            content: "# Attestation\nThis is factual output.".to_string(),
        });
        let state = minimal_test_gateway_state(Some(llm));
        let Json(response) = compliance_letter_handler(
            State(state),
            Some(Json(ComplianceLetterRequest {
                framework: Some("nist".to_string()),
                firm_name: Some("Example LLP".to_string()),
            })),
        )
        .await
        .expect("letter response");
        assert_eq!(response.framework, "nist");
        assert!(
            response
                .markdown
                .contains("Configuration summary only; not legal advice.")
        );
    }

    #[cfg(feature = "libsql")]
    fn test_legal_config() -> crate::config::LegalConfig {
        crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve")
    }

    #[cfg(feature = "libsql")]
    fn test_gateway_state_with_store_workspace_and_legal(
        store: Arc<dyn crate::db::Database>,
        workspace: Arc<Workspace>,
        legal_config: crate::config::LegalConfig,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: Some(workspace),
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(store),
            job_manager: None,
            prompt_queue: None,
            user_id: "test-user".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(
                crate::channels::web::ws::WsConnectionTracker::new(),
            )),
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            startup_time: std::time::Instant::now(),
            legal_config: Some(legal_config),
            runtime_facts: crate::compliance::ComplianceRuntimeFacts::default(),
        })
    }

    #[cfg(feature = "libsql")]
    fn test_gateway_state_with_store_and_workspace(
        store: Arc<dyn crate::db::Database>,
        workspace: Arc<Workspace>,
    ) -> Arc<GatewayState> {
        test_gateway_state_with_store_workspace_and_legal(store, workspace, test_legal_config())
    }

    #[cfg(feature = "libsql")]
    fn test_gateway_state_with_store_workspace_and_chat(
        store: Arc<dyn crate::db::Database>,
        workspace: Arc<Workspace>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: Some(workspace),
            session_manager: Some(Arc::new(SessionManager::new())),
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(store),
            job_manager: None,
            prompt_queue: None,
            user_id: "test-user".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(
                crate::channels::web::ws::WsConnectionTracker::new(),
            )),
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            startup_time: std::time::Instant::now(),
            legal_config: Some(test_legal_config()),
            runtime_facts: crate::compliance::ComplianceRuntimeFacts::default(),
        })
    }

    #[cfg(feature = "libsql")]
    async fn seed_valid_matter(workspace: &Workspace, matter_id: &str) {
        let metadata = format!(
            "matter_id: {matter_id}\nclient: Demo Client\nteam:\n  - Lead Counsel\nconfidentiality: attorney-client-privileged\nadversaries:\n  - Example Co\nretention: follow-firm-policy\n"
        );
        workspace
            .write(&format!("matters/{matter_id}/matter.yaml"), &metadata)
            .await
            .expect("seed matter metadata");
        workspace
            .write(
                &format!("matters/{matter_id}/templates/research_memo.md"),
                "# Research Memo Template\n",
            )
            .await
            .expect("seed research template");
        workspace
            .write(
                &format!("matters/{matter_id}/templates/chronology.md"),
                "# Chronology Template\n",
            )
            .await
            .expect("seed chronology template");
        workspace
            .write(
                &format!("matters/{matter_id}/notes.md"),
                "matter notes content",
            )
            .await
            .expect("seed notes document");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_history_rejects_limit_zero() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(db, workspace);

        let session_manager = state
            .session_manager
            .as_ref()
            .expect("session manager should exist")
            .clone();
        let session = session_manager.get_or_create_session("test-user").await;
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.id
        };

        let err = chat_history_handler(
            State(state),
            Query(HistoryQuery {
                thread_id: Some(thread_id.to_string()),
                limit: Some(0),
                before: None,
            }),
        )
        .await
        .expect_err("limit=0 should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("between 1 and 200"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_history_rejects_limit_above_max() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(db, workspace);

        let session_manager = state
            .session_manager
            .as_ref()
            .expect("session manager should exist")
            .clone();
        let session = session_manager.get_or_create_session("test-user").await;
        let thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.id
        };

        let err = chat_history_handler(
            State(state),
            Query(HistoryQuery {
                thread_id: Some(thread_id.to_string()),
                limit: Some(201),
                before: None,
            }),
        )
        .await
        .expect_err("limit>200 should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("between 1 and 200"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_history_supports_in_memory_and_db_only_threads() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(Arc::clone(&db), workspace);

        let session_manager = state
            .session_manager
            .as_ref()
            .expect("session manager should exist")
            .clone();
        let session = session_manager.get_or_create_session("test-user").await;
        let in_memory_thread_id = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.start_turn("memory prompt");
            thread.complete_turn("memory response");
            thread.id
        };

        let db_only_thread_id = Uuid::new_v4();
        db.ensure_conversation(db_only_thread_id, "gateway", "test-user", None)
            .await
            .expect("ensure db conversation");
        db.add_conversation_message(db_only_thread_id, "user", "db prompt")
            .await
            .expect("seed db user message");
        db.add_conversation_message(db_only_thread_id, "assistant", "db response")
            .await
            .expect("seed db assistant message");

        let Json(in_memory_history) = chat_history_handler(
            State(Arc::clone(&state)),
            Query(HistoryQuery {
                thread_id: None,
                limit: Some(50),
                before: None,
            }),
        )
        .await
        .expect("in-memory history request should succeed");
        assert_eq!(in_memory_history.thread_id, in_memory_thread_id);
        assert_eq!(in_memory_history.turns.len(), 1);
        assert_eq!(in_memory_history.turns[0].user_input, "memory prompt");

        let Json(db_history) = chat_history_handler(
            State(state),
            Query(HistoryQuery {
                thread_id: Some(db_only_thread_id.to_string()),
                limit: Some(50),
                before: None,
            }),
        )
        .await
        .expect("db-only history request should succeed");
        assert_eq!(db_history.thread_id, db_only_thread_id);
        assert_eq!(db_history.turns.len(), 1);
        assert_eq!(db_history.turns[0].user_input, "db prompt");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_history_before_cursor_pagination_unchanged() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(Arc::clone(&db), workspace);

        let thread_id = Uuid::new_v4();
        db.ensure_conversation(thread_id, "gateway", "test-user", None)
            .await
            .expect("ensure db conversation");
        for turn in 1..=3 {
            db.add_conversation_message(thread_id, "user", &format!("turn-{turn}"))
                .await
                .expect("seed db user message");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            db.add_conversation_message(thread_id, "assistant", &format!("resp-{turn}"))
                .await
                .expect("seed db assistant message");
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }

        let Json(first_page) = chat_history_handler(
            State(Arc::clone(&state)),
            Query(HistoryQuery {
                thread_id: Some(thread_id.to_string()),
                limit: Some(2),
                before: None,
            }),
        )
        .await
        .expect("first page should succeed");
        assert_eq!(first_page.turns.len(), 1);
        assert_eq!(first_page.turns[0].user_input, "turn-3");
        let before = first_page
            .oldest_timestamp
            .clone()
            .expect("first page should include oldest timestamp cursor");

        let Json(second_page) = chat_history_handler(
            State(state),
            Query(HistoryQuery {
                thread_id: Some(thread_id.to_string()),
                limit: Some(2),
                before: Some(before),
            }),
        )
        .await
        .expect("second page should succeed");
        assert_eq!(second_page.turns.len(), 1);
        assert_eq!(second_page.turns[0].user_input, "turn-2");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_set_rejects_missing_matter() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let result = matters_active_set_handler(
            State(state),
            Json(SetActiveMatterRequest {
                matter_id: Some("does-not-exist".to_string()),
            }),
        )
        .await;

        let err = result.expect_err("missing matter should be rejected");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        assert!(err.1.contains("not found"));
        assert!(err.1.contains("matter.yaml"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_set_rejects_invalid_metadata() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        workspace
            .write(
                "matters/demo/matter.yaml",
                "matter_id: demo\nclient: Demo Client\n",
            )
            .await
            .expect("seed invalid matter metadata");

        let result = matters_active_set_handler(
            State(state),
            Json(SetActiveMatterRequest {
                matter_id: Some("demo".to_string()),
            }),
        )
        .await;

        let err = result.expect_err("invalid matter metadata should be rejected");
        assert_eq!(err.0, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(err.1.contains("matter.yaml"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_set_accepts_valid_metadata() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        workspace
            .write(
                "matters/demo/matter.yaml",
                "matter_id: demo\nclient: Demo Client\nteam:\n  - Lead Counsel\nconfidentiality: attorney-client-privileged\nadversaries:\n  - Example Co\nretention: follow-firm-policy\n",
            )
            .await
            .expect("seed valid matter metadata");

        let status = matters_active_set_handler(
            State(Arc::clone(&state)),
            Json(SetActiveMatterRequest {
                matter_id: Some("demo".to_string()),
            }),
        )
        .await
        .expect("valid metadata should succeed");
        assert_eq!(status, StatusCode::NO_CONTENT);

        let stored = state
            .store
            .as_ref()
            .expect("store")
            .get_setting("test-user", MATTER_ACTIVE_SETTING)
            .await
            .expect("read setting");
        assert_eq!(
            stored.and_then(|v| v.as_str().map(|s| s.to_string())),
            Some("demo".to_string())
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_get_returns_null_for_malformed_setting() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(Arc::clone(&db), workspace);

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("!!!".to_string()),
            )
            .await
            .expect("set malformed active matter setting");

        let Json(resp) = matters_active_get_handler(State(state))
            .await
            .expect("active matter get should succeed");

        assert_eq!(resp.matter_id, None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_get_returns_null_for_stale_missing_matter() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(Arc::clone(&db), workspace);

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("missing-matter".to_string()),
            )
            .await
            .expect("set stale active matter setting");

        let Json(resp) = matters_active_get_handler(State(state))
            .await
            .expect("active matter get should succeed");

        assert_eq!(resp.matter_id, None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_get_returns_valid_matter_when_metadata_is_valid() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state = test_gateway_state_with_store_and_workspace(Arc::clone(&db), workspace);

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("DEMO".to_string()),
            )
            .await
            .expect("set active matter setting");

        let Json(resp) = matters_active_get_handler(State(state))
            .await
            .expect("active matter get should succeed");

        assert_eq!(resp.matter_id.as_deref(), Some("demo"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_active_set_uses_configured_matter_root() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.matter_root = "casefiles".to_string();
        let state =
            test_gateway_state_with_store_workspace_and_legal(Arc::clone(&db), workspace, legal);

        state
            .workspace
            .as_ref()
            .expect("workspace")
            .write(
                "casefiles/demo/matter.yaml",
                "matter_id: demo\nclient: Demo Client\nteam:\n  - Lead Counsel\nconfidentiality: attorney-client-privileged\nadversaries:\n  - Example Co\nretention: follow-firm-policy\n",
            )
            .await
            .expect("seed valid custom-root matter metadata");

        let status = matters_active_set_handler(
            State(Arc::clone(&state)),
            Json(SetActiveMatterRequest {
                matter_id: Some("demo".to_string()),
            }),
        )
        .await
        .expect("valid metadata under configured root should succeed");
        assert_eq!(status, StatusCode::NO_CONTENT);

        let Json(resp) = matters_active_get_handler(State(state))
            .await
            .expect("active matter get should succeed");
        assert_eq!(resp.matter_id.as_deref(), Some("demo"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_send_includes_active_matter_metadata_when_setting_exists() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let raw_matter = "Acme v. Foo!!!";
        let expected = crate::legal::policy::sanitize_matter_id(raw_matter);
        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String(raw_matter.to_string()),
            )
            .await
            .expect("set active matter setting");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let (status, _resp) = chat_send_handler(
            State(Arc::clone(&state)),
            Json(SendMessageRequest {
                content: "draft a memo".to_string(),
                thread_id: Some("thread-123".to_string()),
            }),
        )
        .await
        .expect("chat send should succeed");

        assert_eq!(status, StatusCode::ACCEPTED);

        let sent = rx.recv().await.expect("message should be forwarded");
        assert_eq!(sent.thread_id.as_deref(), Some("thread-123"));
        assert_eq!(
            sent.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("thread-123")
        );
        assert_eq!(
            sent.metadata.get("active_matter").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_send_sets_active_matter_metadata_to_null_when_missing() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let (status, _resp) = chat_send_handler(
            State(Arc::clone(&state)),
            Json(SendMessageRequest {
                content: "hello".to_string(),
                thread_id: None,
            }),
        )
        .await
        .expect("chat send should succeed");
        assert_eq!(status, StatusCode::ACCEPTED);

        let sent = rx.recv().await.expect("message should be forwarded");
        assert_eq!(
            sent.metadata.get("active_matter"),
            Some(&serde_json::Value::Null)
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_approval_includes_active_matter_metadata_when_setting_exists() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let raw_matter = "Acme v. Foo!!!";
        let expected = crate::legal::policy::sanitize_matter_id(raw_matter);
        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String(raw_matter.to_string()),
            )
            .await
            .expect("set active matter setting");

        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        *state.msg_tx.write().await = Some(tx);

        let request_id = Uuid::new_v4();
        let (status, _resp) = chat_approval_handler(
            State(Arc::clone(&state)),
            Json(ApprovalRequest {
                request_id: request_id.to_string(),
                action: "approve".to_string(),
                thread_id: Some("thread-approval".to_string()),
            }),
        )
        .await
        .expect("approval send should succeed");
        assert_eq!(status, StatusCode::ACCEPTED);

        let sent = rx.recv().await.expect("message should be forwarded");
        assert_eq!(sent.thread_id.as_deref(), Some("thread-approval"));
        assert_eq!(
            sent.metadata.get("thread_id").and_then(|v| v.as_str()),
            Some("thread-approval")
        );
        assert_eq!(
            sent.metadata.get("active_matter").and_then(|v| v.as_str()),
            Some(expected.as_str())
        );

        let submission: crate::agent::submission::Submission =
            serde_json::from_str(&sent.content).expect("approval payload should parse");
        match submission {
            crate::agent::submission::Submission::ExecApproval {
                request_id: parsed_id,
                approved,
                always,
            } => {
                assert_eq!(parsed_id, request_id);
                assert!(approved);
                assert!(!always);
            }
            other => panic!("expected ExecApproval payload, got {:?}", other),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_new_thread_binds_active_matter_to_conversation() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(Arc::clone(&db), workspace);

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("DEMO".to_string()),
            )
            .await
            .expect("set active matter setting");

        let Json(resp) = chat_new_thread_handler(State(Arc::clone(&state)))
            .await
            .expect("new thread should succeed");
        assert_eq!(resp.matter_id.as_deref(), Some("demo"));

        let bound = state
            .store
            .as_ref()
            .expect("store")
            .get_conversation_matter_id(resp.id, "test-user")
            .await
            .expect("conversation lookup should succeed");
        assert_eq!(bound.as_deref(), Some("demo"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_threads_filter_returns_only_requested_matter() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_workspace_and_chat(Arc::clone(&db), workspace);

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("demo".to_string()),
            )
            .await
            .expect("set active matter setting");
        let Json(demo_thread) = chat_new_thread_handler(State(Arc::clone(&state)))
            .await
            .expect("demo thread create should succeed");

        state
            .store
            .as_ref()
            .expect("store")
            .set_setting(
                "test-user",
                MATTER_ACTIVE_SETTING,
                &serde_json::Value::String("other".to_string()),
            )
            .await
            .expect("set active matter setting");
        let Json(other_thread) = chat_new_thread_handler(State(Arc::clone(&state)))
            .await
            .expect("other thread create should succeed");

        let Json(filtered) = chat_threads_handler(
            State(Arc::clone(&state)),
            Query(ThreadListQuery {
                matter_id: Some("demo".to_string()),
            }),
        )
        .await
        .expect("filtered threads call should succeed");

        assert!(
            filtered.assistant_thread.is_none(),
            "matter-filtered thread list should not include assistant thread"
        );
        assert!(
            filtered
                .threads
                .iter()
                .any(|thread| thread.id == demo_thread.id),
            "expected demo thread in filtered result"
        );
        assert!(
            filtered
                .threads
                .iter()
                .all(|thread| thread.id != other_thread.id),
            "other matter thread should be excluded"
        );
        assert!(
            filtered
                .threads
                .iter()
                .all(|thread| thread.matter_id.as_deref() == Some("demo")),
            "all returned threads should include demo matter id"
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn chat_threads_filter_rejects_empty_after_sanitization() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(Arc::clone(&db), workspace);

        let err = chat_threads_handler(
            State(state),
            Query(ThreadListQuery {
                matter_id: Some("!!!".to_string()),
            }),
        )
        .await
        .expect_err("invalid matter filter should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(
            err.1.contains("empty after sanitization"),
            "unexpected error message: {}",
            err.1
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_creates_scaffold_and_sets_active() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let (status, Json(response)) = matters_create_handler(
            State(Arc::clone(&state)),
            Json(CreateMatterRequest {
                matter_id: "Acme v. Foo".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "attorney-client-privileged".to_string(),
                retention: "follow-firm-policy".to_string(),
                jurisdiction: Some("SDNY / Delaware".to_string()),
                practice_area: Some("commercial litigation".to_string()),
                opened_date: Some("2024-03-15".to_string()),
                opened_at: None,
                team: vec!["Lead Counsel".to_string()],
                adversaries: vec!["Foo LLC".to_string()],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect("create matter should succeed");

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(response.active_matter_id, "acme-v--foo");
        assert_eq!(response.matter.id, "acme-v--foo");
        assert_eq!(
            response.matter.jurisdiction.as_deref(),
            Some("SDNY / Delaware")
        );
        assert_eq!(
            response.matter.practice_area.as_deref(),
            Some("commercial litigation")
        );
        assert_eq!(response.matter.opened_date.as_deref(), Some("2024-03-15"));

        let metadata = workspace
            .read("matters/acme-v--foo/matter.yaml")
            .await
            .expect("matter.yaml should exist");
        let parsed: crate::legal::matter::MatterMetadata =
            serde_yml::from_str(&metadata.content).expect("matter.yaml should parse");
        assert_eq!(parsed.matter_id, "acme-v--foo");
        assert_eq!(parsed.jurisdiction.as_deref(), Some("SDNY / Delaware"));
        assert_eq!(
            parsed.practice_area.as_deref(),
            Some("commercial litigation")
        );
        assert_eq!(parsed.opened_date.as_deref(), Some("2024-03-15"));
        let workflow = workspace
            .read("matters/acme-v--foo/workflows/intake_checklist.md")
            .await
            .expect("intake checklist should exist");
        assert!(workflow.content.contains("conflict check"));
        let deadlines = workspace
            .read("matters/acme-v--foo/deadlines/calendar.md")
            .await
            .expect("deadlines file should exist");
        assert!(deadlines.content.contains("Deadline / Event"));
        let legal_memo_template = workspace
            .read("matters/acme-v--foo/templates/legal_memo.md")
            .await
            .expect("legal memo template should exist");
        assert!(legal_memo_template.content.contains("## Facts (Cited)"));

        let stored = state
            .store
            .as_ref()
            .expect("store")
            .get_setting("test-user", MATTER_ACTIVE_SETTING)
            .await
            .expect("read setting");
        assert_eq!(
            stored.and_then(|v| v.as_str().map(|s| s.to_string())),
            Some("acme-v--foo".to_string())
        );
        let events = crate::legal::audit::test_events_snapshot();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "matter_created")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_list_includes_optional_metadata_fields() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let _ = matters_create_handler(
            State(Arc::clone(&state)),
            Json(CreateMatterRequest {
                matter_id: "Acme v. Foo".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "attorney-client-privileged".to_string(),
                retention: "follow-firm-policy".to_string(),
                jurisdiction: Some("SDNY / Delaware".to_string()),
                practice_area: Some("commercial litigation".to_string()),
                opened_date: Some("2024-03-15".to_string()),
                opened_at: None,
                team: vec!["Lead Counsel".to_string()],
                adversaries: vec!["Foo LLC".to_string()],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect("create matter should succeed");

        let Json(list) = matters_list_handler(State(state))
            .await
            .expect("matters list should succeed");
        assert_eq!(list.matters.len(), 1);
        let matter = &list.matters[0];
        assert_eq!(matter.id, "acme-v--foo");
        assert_eq!(matter.jurisdiction.as_deref(), Some("SDNY / Delaware"));
        assert_eq!(
            matter.practice_area.as_deref(),
            Some("commercial litigation")
        );
        assert_eq!(matter.opened_date.as_deref(), Some("2024-03-15"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_duplicate() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let _created = matters_create_handler(
            State(Arc::clone(&state)),
            Json(CreateMatterRequest {
                matter_id: "demo".to_string(),
                client: "Demo".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect("first create should succeed");

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "demo".to_string(),
                client: "Demo".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("duplicate should fail");

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("already exists"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_invalid_opened_date() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "demo".to_string(),
                client: "Demo".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: Some("03/15/2024".to_string()),
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("invalid opened_date should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("YYYY-MM-DD"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_overlong_optional_metadata_fields() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "demo".to_string(),
                client: "Demo".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: Some("X".repeat(257)),
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("overlong jurisdiction should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("jurisdiction"));
        assert!(err.1.contains("at most"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_empty_after_sanitize() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "!!!".to_string(),
                client: "Demo".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("invalid matter id should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("empty after sanitization"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn intake_conflict_check_returns_structured_hits() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        db.seed_matter_parties("existing-matter", "Acme Corp", &[], None)
            .await
            .expect("seed matter parties");
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.require_matter_context = false;
        legal.conflict_check_enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = matters_conflict_check_handler(
            State(state),
            Json(MatterIntakeConflictCheckRequest {
                matter_id: "new-matter".to_string(),
                client_names: vec!["Acme Corp".to_string()],
                adversary_names: vec!["Other Party".to_string()],
            }),
        )
        .await
        .expect("intake conflict check should succeed");

        assert!(resp.matched);
        assert_eq!(resp.matter_id, "new-matter");
        assert_eq!(resp.checked_parties.len(), 2);
        assert!(!resp.hits.is_empty());
        assert!(resp.hits.iter().any(|hit| hit.party == "Acme Corp"));

        let events = crate::legal::audit::test_events_snapshot();
        let intake_event = events
            .iter()
            .find(|event| event.event_type == "matter_intake_conflict_check")
            .expect("expected intake conflict audit event");
        assert_eq!(intake_event.details["matched"], true);
        assert_eq!(intake_event.details["matter_id"], "new-matter");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn intake_conflict_check_rejects_empty_client_names() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_conflict_check_handler(
            State(state),
            Json(MatterIntakeConflictCheckRequest {
                matter_id: "new-matter".to_string(),
                client_names: vec!["   ".to_string()],
                adversary_names: vec![],
            }),
        )
        .await
        .expect_err("empty client list should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("client_names"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn intake_conflict_check_rejects_excessive_client_names() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);
        let client_names: Vec<String> = (0..=MAX_INTAKE_CONFLICT_PARTIES)
            .map(|idx| format!("Client {idx}"))
            .collect();

        let err = matters_conflict_check_handler(
            State(state),
            Json(MatterIntakeConflictCheckRequest {
                matter_id: "new-matter".to_string(),
                client_names,
                adversary_names: vec![],
            }),
        )
        .await
        .expect_err("oversized client list should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("client_names"));
        assert!(err.1.contains("at most"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn intake_conflict_check_respects_disabled_policy() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.conflict_check_enabled = false;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let err = matters_conflict_check_handler(
            State(state),
            Json(MatterIntakeConflictCheckRequest {
                matter_id: "new-matter".to_string(),
                client_names: vec!["Acme Corp".to_string()],
                adversary_names: vec![],
            }),
        )
        .await
        .expect_err("disabled policy should reject");

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("disabled"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_reindex_backfills_graph_and_emits_audit_event() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "matters/demo/matter.yaml",
                r#"
matter_id: demo
client: Demo Client
team:
  - Lead Counsel
confidentiality: attorney-client-privileged
adversaries:
  - Foo Industries
retention: follow-firm-policy
opened_at: 2026-02-28
"#,
            )
            .await
            .expect("seed matter metadata");
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Example Adverse Party","aliases":["Example Co"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.conflict_check_enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(
            Arc::clone(&db),
            Arc::clone(&workspace),
            legal,
        );

        let Json(resp) = matters_conflicts_reindex_handler(State(state))
            .await
            .expect("reindex should succeed");

        assert_eq!(resp.status, "ok");
        assert_eq!(resp.report.seeded_matters, 1);
        assert_eq!(resp.report.global_conflicts_seeded, 1);
        assert_eq!(resp.report.global_aliases_seeded, 1);

        let alias_hits = db
            .find_conflict_hits_for_names(&["Example Co".to_string()], 20)
            .await
            .expect("query seeded alias");
        assert!(
            alias_hits.iter().any(|hit| {
                hit.matter_id == crate::legal::matter::GLOBAL_CONFLICT_GRAPH_MATTER_ID
            })
        );

        let events = crate::legal::audit::test_events_snapshot();
        assert!(events.iter().any(|event| {
            event.event_type == "conflict_graph_reindexed"
                && event.details["seeded_matters"] == 1
                && event.details["global_conflicts_seeded"] == 1
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_reindex_respects_disabled_policy() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.conflict_check_enabled = false;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let err = matters_conflicts_reindex_handler(State(state))
            .await
            .expect_err("disabled policy should reject reindex");
        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("disabled"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_requires_conflict_decision_when_hits_exist() {
        let (db, _tmp) = crate::testing::test_db().await;
        db.seed_matter_parties("existing-matter", "Acme Corp", &[], None)
            .await
            .expect("seed matter parties");
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "new-matter".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("missing conflict decision should fail");

        assert_eq!(err.0, StatusCode::CONFLICT);
        let body: serde_json::Value =
            serde_json::from_str(&err.1).expect("conflict body should be json");
        assert_eq!(body["conflict_required"], true);
        assert!(body["hits"].as_array().is_some_and(|hits| !hits.is_empty()));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_declined_records_audit_and_blocks_creation() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        db.seed_matter_parties("existing-matter", "Acme Corp", &[], None)
            .await
            .expect("seed matter parties");
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "new-matter".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries: vec![],
                conflict_decision: Some(ConflictDecision::Declined),
                conflict_note: Some("Escalated to conflicts counsel".to_string()),
            }),
        )
        .await
        .expect_err("declined decision should block creation");

        assert_eq!(err.0, StatusCode::CONFLICT);
        let body: serde_json::Value =
            serde_json::from_str(&err.1).expect("declined body should be json");
        assert_eq!(body["decision"], "declined");
        let created = workspace.read("matters/new-matter/matter.yaml").await;
        assert!(matches!(
            created,
            Err(crate::error::WorkspaceError::DocumentNotFound { .. })
        ));

        let events = crate::legal::audit::test_events_snapshot();
        let decision_event = events
            .iter()
            .find(|event| event.event_type == "conflict_clearance_decision")
            .expect("expected conflict_clearance_decision event");
        assert_eq!(decision_event.details["decision"], "declined");
        assert_eq!(decision_event.details["source"], "create_flow");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_waived_records_and_proceeds() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        db.seed_matter_parties("existing-matter", "Acme Corp", &[], None)
            .await
            .expect("seed matter parties");
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let (status, Json(resp)) = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "new-matter".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: Some("2026-02-28".to_string()),
                opened_at: None,
                team: vec![],
                adversaries: vec!["Other Party".to_string()],
                conflict_decision: Some(ConflictDecision::Waived),
                conflict_note: Some("Waived after documented informed consent".to_string()),
            }),
        )
        .await
        .expect("waived decision should allow creation");

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(resp.matter.id, "new-matter");
        workspace
            .read("matters/new-matter/matter.yaml")
            .await
            .expect("matter yaml should exist");

        let hits = db
            .find_conflict_hits_for_names(&["Acme Corp".to_string()], 20)
            .await
            .expect("conflict search should succeed");
        assert!(
            hits.iter().any(|hit| hit.matter_id == "new-matter"),
            "seed_matter_parties should register new matter parties"
        );

        let events = crate::legal::audit::test_events_snapshot();
        assert!(events.iter().any(|event| {
            event.event_type == "conflict_clearance_decision"
                && event.details["decision"] == "waived"
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_excessive_adversaries() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);
        let adversaries: Vec<String> = (0..=MAX_INTAKE_CONFLICT_PARTIES)
            .map(|idx| format!("Adverse Party {idx}"))
            .collect();

        let err = matters_create_handler(
            State(state),
            Json(CreateMatterRequest {
                matter_id: "new-matter".to_string(),
                client: "Acme Corp".to_string(),
                confidentiality: "privileged".to_string(),
                retention: "policy".to_string(),
                jurisdiction: None,
                practice_area: None,
                opened_date: None,
                opened_at: None,
                team: vec![],
                adversaries,
                conflict_decision: None,
                conflict_note: None,
            }),
        )
        .await
        .expect_err("oversized adversary list should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("adversaries"));
        assert!(err.1.contains("at most"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_returns_hit_for_matching_entry() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Alpha Holdings","aliases":["Alpha"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.require_matter_context = false;
        legal.conflict_check_enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "Draft strategy for Alpha Holdings".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect("conflicts check should succeed");

        assert!(resp.matched);
        assert_eq!(resp.conflict.as_deref(), Some("Alpha Holdings"));
        assert!(
            resp.hits.is_empty(),
            "legacy file fallback should not return db hits"
        );

        let events = crate::legal::audit::test_events_snapshot();
        let event = events
            .iter()
            .find(|entry| {
                entry.event_type == "matter_conflict_check"
                    && entry.details.get("source").and_then(|v| v.as_str())
                        == Some("manual_text_check")
                    && entry.details.get("conflict").and_then(|v| v.as_str())
                        == Some("Alpha Holdings")
            })
            .expect("expected manual conflict check audit event");
        assert_eq!(event.details["matched"], true);
        assert_eq!(event.details["conflict"], "Alpha Holdings");
        assert_eq!(event.details["source"], "manual_text_check");
        assert!(
            event.details["text_preview"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
        assert!(events.iter().any(|entry| {
            entry.event_type == "conflict_detected"
                && entry.details.get("source").and_then(|v| v.as_str()) == Some("manual_text_check")
        }));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_rejects_oversized_text_payload() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.require_matter_context = false;
        legal.conflict_check_enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let oversized = "A".repeat(MAX_CONFLICT_CHECK_TEXT_LEN + 1);
        let err = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: oversized,
                matter_id: None,
            }),
        )
        .await
        .expect_err("oversized payload should be rejected");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("at most"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_returns_db_hits_context() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        db.seed_matter_parties("existing-matter", "Acme Corp", &[], None)
            .await
            .expect("seed matter parties");
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.require_matter_context = false;
        legal.conflict_check_enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "Please analyze exposure for Acme Corp".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect("conflicts check should succeed");

        assert!(resp.matched);
        assert_eq!(resp.conflict.as_deref(), Some("Acme Corp"));
        assert!(!resp.hits.is_empty());
        assert!(
            resp.hits
                .iter()
                .any(|hit| hit.matter_id == "existing-matter")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_skips_file_fallback_when_db_authoritative_mode_enabled() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Fallback Party","aliases":["Fallback Co"]}]"#,
            )
            .await
            .expect("seed fallback conflicts");

        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.require_matter_context = false;
        legal.conflict_check_enabled = true;
        legal.conflict_file_fallback_enabled = false;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "Review communications with Fallback Party".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect("manual conflict check should succeed");

        assert!(!resp.matched);
        assert!(resp.conflict.is_none());
        assert!(resp.hits.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_rejects_empty_text() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let err = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "   ".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect_err("empty text should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("must not be empty"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_respects_disabled_config() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.conflict_check_enabled = false;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let err = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "Alpha".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect_err("disabled conflict check should fail");

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("disabled"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_requires_active_matter_when_policy_enabled() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.enabled = true;
        legal.conflict_check_enabled = true;
        legal.require_matter_context = true;
        legal.active_matter = None;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let err = matters_conflicts_check_handler(
            State(state),
            Json(MatterConflictCheckRequest {
                text: "Alpha".to_string(),
                matter_id: None,
            }),
        )
        .await
        .expect_err("missing matter context should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("Active matter"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_audit_list_returns_empty_when_missing() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = legal_audit_list_handler(State(state), Query(LegalAuditQuery::default()))
            .await
            .expect("empty DB list should not error");

        assert!(resp.events.is_empty());
        assert_eq!(resp.total, 0);
        assert_eq!(resp.next_offset, None);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_audit_list_supports_filters_and_paging() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));

        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        let state =
            test_gateway_state_with_store_workspace_and_legal(Arc::clone(&db), workspace, legal);
        let store = state.store.as_ref().expect("store should exist");
        for idx in 1..=4 {
            let severity = if idx % 2 == 0 {
                crate::db::AuditSeverity::Warn
            } else {
                crate::db::AuditSeverity::Info
            };
            store
                .append_audit_event(
                    &state.user_id,
                    &crate::db::AppendAuditEventParams {
                        event_type: "approval_required".to_string(),
                        actor: "gateway".to_string(),
                        matter_id: Some("demo".to_string()),
                        severity,
                        details: serde_json::json!({ "id": idx }),
                    },
                )
                .await
                .expect("append audit event");
        }

        let Json(resp) = legal_audit_list_handler(
            State(Arc::clone(&state)),
            Query(LegalAuditQuery {
                limit: Some(1),
                offset: Some(0),
                event_type: Some("approval_required".to_string()),
                matter_id: Some("demo".to_string()),
                severity: Some("warn".to_string()),
                since: None,
                until: None,
                from: None,
                to: None,
            }),
        )
        .await
        .expect("audit list should succeed");

        assert_eq!(resp.total, 2);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.next_offset, Some(1));
        assert_eq!(resp.events[0].event_type, "approval_required");
        assert_eq!(resp.events[0].matter_id.as_deref(), Some("demo"));
        assert_eq!(resp.events[0].severity, "warn");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_audit_list_filters_since_until() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));

        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        let state =
            test_gateway_state_with_store_workspace_and_legal(Arc::clone(&db), workspace, legal);
        let store = state.store.as_ref().expect("store should exist");
        store
            .append_audit_event(
                &state.user_id,
                &crate::db::AppendAuditEventParams {
                    event_type: "matter_created".to_string(),
                    actor: "gateway".to_string(),
                    matter_id: Some("demo".to_string()),
                    severity: crate::db::AuditSeverity::Info,
                    details: serde_json::json!({ "step": 1 }),
                },
            )
            .await
            .expect("append");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let checkpoint = Utc::now().to_rfc3339();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        store
            .append_audit_event(
                &state.user_id,
                &crate::db::AppendAuditEventParams {
                    event_type: "matter_closed".to_string(),
                    actor: "gateway".to_string(),
                    matter_id: Some("demo".to_string()),
                    severity: crate::db::AuditSeverity::Critical,
                    details: serde_json::json!({ "step": 2 }),
                },
            )
            .await
            .expect("append");

        let Json(resp) = legal_audit_list_handler(
            State(state),
            Query(LegalAuditQuery {
                limit: Some(50),
                offset: Some(0),
                event_type: None,
                matter_id: Some("demo".to_string()),
                severity: None,
                since: Some(checkpoint),
                until: None,
                from: None,
                to: None,
            }),
        )
        .await
        .expect("audit list should succeed");

        assert_eq!(resp.total, 1);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.events[0].event_type, "matter_closed");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_documents_excludes_templates_by_default() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        workspace
            .write("matters/demo/templates-archive/note.md", "archive note")
            .await
            .expect("seed templates-archive sibling");
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let Json(resp) = matter_documents_handler(
            State(state),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("documents request should succeed");

        assert_eq!(resp.matter_id, "demo");
        assert!(
            !resp
                .documents
                .iter()
                .any(|doc| doc.path.contains("/templates/"))
        );
        assert!(
            resp.documents
                .iter()
                .any(|doc| doc.path == "matters/demo/notes.md")
        );
        assert!(
            resp.documents
                .iter()
                .any(|doc| doc.path == "matters/demo/templates-archive/note.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_documents_includes_templates_when_requested() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let Json(resp) = matter_documents_handler(
            State(state),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery {
                include_templates: Some(true),
            }),
        )
        .await
        .expect("documents request should succeed");

        assert!(
            resp.documents
                .iter()
                .any(|doc| doc.path == "matters/demo/templates/research_memo.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_templates_list_returns_expected_entries() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let Json(resp) = matter_templates_handler(State(state), Path("demo".to_string()))
            .await
            .expect("templates request should succeed");

        assert_eq!(resp.matter_id, "demo");
        assert_eq!(resp.templates.len(), 2);
        assert_eq!(resp.templates[0].name, "chronology.md");
        assert_eq!(resp.templates[1].name, "research_memo.md");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_documents_backfill_incrementally_syncs_new_workspace_files() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let Json(initial) = matter_documents_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("initial documents request should succeed");
        assert!(
            initial
                .documents
                .iter()
                .any(|doc| doc.path == "matters/demo/notes.md")
        );

        workspace
            .write(
                "matters/demo/discovery/new-evidence.md",
                "new evidence notes",
            )
            .await
            .expect("seed new workspace document");

        let Json(updated) = matter_documents_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("updated documents request should succeed");
        assert!(
            updated
                .documents
                .iter()
                .any(|doc| doc.path == "matters/demo/discovery/new-evidence.md")
        );

        let linked = db
            .list_matter_documents_db("test-user", "demo")
            .await
            .expect("matter documents query");
        assert!(
            linked
                .iter()
                .any(|doc| doc.path == "matters/demo/discovery/new-evidence.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_templates_backfill_incrementally_syncs_new_workspace_templates() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let Json(initial) =
            matter_templates_handler(State(Arc::clone(&state)), Path("demo".to_string()))
                .await
                .expect("initial templates request should succeed");
        assert_eq!(initial.templates.len(), 2);

        workspace
            .write(
                "matters/demo/templates/witness_outline.md",
                "# Witness Outline Template\n",
            )
            .await
            .expect("seed new workspace template");

        let Json(updated) =
            matter_templates_handler(State(Arc::clone(&state)), Path("demo".to_string()))
                .await
                .expect("updated templates request should succeed");
        assert!(
            updated
                .templates
                .iter()
                .any(|template| template.name == "witness_outline.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_documents_backfill_does_not_duplicate_initial_versions() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let _ = matter_documents_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("first documents request should succeed");

        workspace
            .write(
                "matters/demo/discovery/new-evidence.md",
                "new evidence notes",
            )
            .await
            .expect("seed new workspace document");

        let _ = matter_documents_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("second documents request should succeed");

        let _ = matter_documents_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterDocumentsQuery::default()),
        )
        .await
        .expect("third documents request should succeed");

        let docs = db
            .list_matter_documents_db("test-user", "demo")
            .await
            .expect("matter documents query");
        for doc in docs {
            let versions = db
                .list_document_versions("test-user", doc.id)
                .await
                .expect("document versions query");
            assert_eq!(
                versions.len(),
                1,
                "document {} should have exactly one initial version",
                doc.path
            );
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_template_apply_creates_timestamped_draft() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let (status, Json(resp)) = matter_template_apply_handler(
            State(state),
            Path("demo".to_string()),
            Json(MatterTemplateApplyRequest {
                template_name: "chronology.md".to_string(),
            }),
        )
        .await
        .expect("apply template should succeed");

        assert_eq!(status, StatusCode::CREATED);
        let re = Regex::new(r"^matters/demo/drafts/chronology-\d{8}-\d{6}(-\d+)?\.md$")
            .expect("valid regex");
        assert!(
            re.is_match(&resp.path),
            "unexpected draft path: {}",
            resp.path
        );
        let written = workspace
            .read(&resp.path)
            .await
            .expect("draft should exist");
        assert!(written.content.contains("# Chronology Template"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_template_apply_avoids_overwrite_collisions() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let matter_prefix = "matters/demo";
        let fixed_ts = "20260226-120000";

        let first = choose_template_apply_destination(
            workspace.as_ref(),
            matter_prefix,
            "chronology.md",
            fixed_ts,
        )
        .await
        .expect("first destination");
        workspace
            .write(&first, "existing draft")
            .await
            .expect("seed collision");

        let second = choose_template_apply_destination(
            workspace.as_ref(),
            matter_prefix,
            "chronology.md",
            fixed_ts,
        )
        .await
        .expect("second destination");

        assert_ne!(first, second);
        assert!(
            second.ends_with("-2.md"),
            "expected -2 suffix, got {}",
            second
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn documents_generate_creates_matter_link_and_version() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        // Ensure matter + client rows exist for docgen context.
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let Json(templates_resp) =
            matter_templates_handler(State(Arc::clone(&state)), Path("demo".to_string()))
                .await
                .expect("templates request should succeed");
        let template_id = templates_resp
            .templates
            .iter()
            .find(|template| template.name == "chronology.md")
            .and_then(|template| template.id.clone())
            .expect("template id should exist");

        let (status, Json(resp)) = documents_generate_handler(
            State(Arc::clone(&state)),
            Json(GenerateDocumentRequest {
                template_id,
                matter_id: "demo".to_string(),
                extra: serde_json::json!({ "event": "hearing" }),
                display_name: Some("Chronology Draft".to_string()),
                category: Some("internal".to_string()),
                label: Some("draft".to_string()),
            }),
        )
        .await
        .expect("generate request should succeed");

        assert_eq!(status, StatusCode::CREATED);
        assert!(resp.path.starts_with("matters/demo/drafts/chronology-"));

        let generated = workspace
            .read(&resp.path)
            .await
            .expect("generated doc exists");
        assert!(
            generated.content.contains("# Chronology Template"),
            "rendered content should contain template body"
        );

        let matter_docs = db
            .list_matter_documents_db("test-user", "demo")
            .await
            .expect("matter documents query");
        let linked = matter_docs
            .iter()
            .find(|doc| doc.id.to_string() == resp.matter_document_id)
            .expect("generated link should exist");
        assert_eq!(linked.display_name, "Chronology Draft");

        let versions = db
            .list_document_versions("test-user", linked.id)
            .await
            .expect("document versions query");
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].label, "draft");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_deadlines_handler_parses_calendar_rows() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let today = Utc::now().date_naive();
        let past = (today - chrono::TimeDelta::days(1)).to_string();
        let upcoming = (today + chrono::TimeDelta::days(5)).to_string();
        let followup = (today + chrono::TimeDelta::days(8)).to_string();

        workspace
            .write(
                "matters/demo/deadlines/calendar.md",
                &format!(
                    "# Deadlines\n\n| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n| {past} | Initial disclosure due | Lead Counsel | open | FRCP 26 |\n| {upcoming} | File reply brief | Associate | drafting | court order |\n| {followup} | Submit witness list |  | open | scheduling order |\n"
                ),
            )
            .await
            .expect("seed deadlines calendar");

        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let Json(resp) = matter_deadlines_handler(State(state), Path("demo".to_string()))
            .await
            .expect("deadlines handler should succeed");

        assert_eq!(resp.matter_id, "demo");
        assert_eq!(resp.deadlines.len(), 3);
        assert!(resp.deadlines[0].is_overdue);
        assert!(!resp.deadlines[1].is_overdue);
        assert_eq!(resp.deadlines[1].title, "File reply brief");
        assert_eq!(resp.deadlines[2].title, "Submit witness list");
        assert_eq!(resp.deadlines[2].owner, None);
        assert_eq!(resp.deadlines[2].status.as_deref(), Some("open"));
        assert_eq!(
            resp.deadlines[2].source.as_deref(),
            Some("scheduling order")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_deadlines_db_entries_prefer_over_workspace_calendar() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        workspace
            .write(
                "matters/demo/deadlines/calendar.md",
                "| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n| 2030-01-01 | Legacy calendar row | Team | open | file |\n",
            )
            .await
            .expect("seed legacy calendar");
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let due_at = (Utc::now() + chrono::TimeDelta::days(7)).to_rfc3339();
        let (status, Json(created)) = matter_deadlines_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateMatterDeadlineRequest {
                title: "File opposition brief".to_string(),
                deadline_type: "filing".to_string(),
                due_at: due_at.clone(),
                completed_at: None,
                reminder_days: vec![3],
                rule_ref: Some("FRCP 56(c)(1)".to_string()),
                computed_from: None,
                task_id: None,
            }),
        )
        .await
        .expect("create deadline should succeed");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.title, "File opposition brief");

        let Json(resp) = matter_deadlines_handler(State(state), Path("demo".to_string()))
            .await
            .expect("deadlines handler should succeed");
        assert_eq!(resp.deadlines.len(), 1);
        assert_eq!(resp.deadlines[0].title, "File opposition brief");
        assert_eq!(resp.deadlines[0].source.as_deref(), Some("FRCP 56(c)(1)"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_court_rules_and_compute_deadline() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state = test_gateway_state_with_store_and_workspace(db, workspace);

        let Json(rules_resp) = legal_court_rules_handler()
            .await
            .expect("rules handler should succeed");
        assert!(rules_resp.rules.iter().any(|rule| rule.id == "frcp_12_a_1"));

        let Json(computed) = matter_deadlines_compute_handler(
            State(state),
            Path("demo".to_string()),
            Json(MatterDeadlineComputeRequest {
                rule_id: "frcp_12_a_1".to_string(),
                trigger_date: "2026-03-02".to_string(),
                title: Some("Response due".to_string()),
                reminder_days: vec![7, 3],
                computed_from: None,
                task_id: None,
            }),
        )
        .await
        .expect("compute handler should succeed");
        assert_eq!(computed.rule.id, "frcp_12_a_1");
        assert!(
            computed.deadline.due_at.starts_with("2026-03-23T"),
            "unexpected due_at {}",
            computed.deadline.due_at
        );
        assert_eq!(computed.deadline.reminder_days, vec![3, 7]);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_deadline_delete_disables_reminder_routines() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let due_at = (Utc::now() + chrono::TimeDelta::days(10)).to_rfc3339();
        let (_status, Json(created)) = matter_deadlines_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateMatterDeadlineRequest {
                title: "Serve discovery requests".to_string(),
                deadline_type: "discovery_cutoff".to_string(),
                due_at,
                completed_at: None,
                reminder_days: vec![1, 3],
                rule_ref: None,
                computed_from: None,
                task_id: None,
            }),
        )
        .await
        .expect("create deadline should succeed");

        let deadline_id = Uuid::parse_str(&created.id).expect("deadline uuid");
        let prefix = deadline_reminder_prefix("demo", deadline_id);

        let before_delete = db
            .list_routines("test-user")
            .await
            .expect("list routines before delete");
        let active_count = before_delete
            .iter()
            .filter(|routine| routine.name.starts_with(&prefix) && routine.enabled)
            .count();
        assert_eq!(active_count, 2);

        let status = matter_deadlines_delete_handler(
            State(state),
            Path(("demo".to_string(), created.id.clone())),
        )
        .await
        .expect("delete deadline should succeed");
        assert_eq!(status, StatusCode::NO_CONTENT);

        let after_delete = db
            .list_routines("test-user")
            .await
            .expect("list routines after delete");
        let routines: Vec<_> = after_delete
            .into_iter()
            .filter(|routine| routine.name.starts_with(&prefix))
            .collect();
        assert_eq!(routines.len(), 2);
        assert!(routines.iter().all(|routine| !routine.enabled));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_dashboard_reports_workflow_scorecard() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let today = Utc::now().date_naive();
        let upcoming = (today + chrono::TimeDelta::days(7)).to_string();
        let overdue = (today - chrono::TimeDelta::days(2)).to_string();

        workspace
            .write("matters/demo/drafts/first-brief.md", "Draft body")
            .await
            .expect("seed draft");
        workspace
            .write(
                "matters/demo/workflows/intake_checklist.md",
                "- [x] Engagement confirmed\n- [ ] Conflict memo attached\n",
            )
            .await
            .expect("seed intake checklist");
        workspace
            .write(
                "matters/demo/workflows/review_and_filing_checklist.md",
                "- [x] Citation format pass complete\n- [ ] Partner sign-off recorded\n",
            )
            .await
            .expect("seed review checklist");
        workspace
            .write(
                "matters/demo/deadlines/calendar.md",
                &format!(
                    "| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n| {overdue} | Serve disclosures | Team | open | docket |\n| {upcoming} | File opposition | Team | open | order |\n"
                ),
            )
            .await
            .expect("seed deadlines");

        let state = test_gateway_state_with_store_and_workspace(db, workspace);
        let Json(resp) = matter_dashboard_handler(State(state), Path("demo".to_string()))
            .await
            .expect("dashboard handler should succeed");

        assert_eq!(resp.matter_id, "demo");
        assert_eq!(resp.template_count, 2);
        assert_eq!(resp.draft_count, 1);
        assert_eq!(resp.checklist_completed, 2);
        assert_eq!(resp.checklist_total, 4);
        assert_eq!(resp.overdue_deadlines, 1);
        assert_eq!(resp.upcoming_deadlines_14d, 1);
        assert_eq!(
            resp.next_deadline.as_ref().map(|item| item.date.as_str()),
            Some(upcoming.as_str())
        );
        assert!(resp.document_count >= 6);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_filing_package_creates_export_index() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        workspace
            .write(
                "matters/demo/workflows/intake_checklist.md",
                "- [x] Intake complete\n",
            )
            .await
            .expect("seed checklist");
        workspace
            .write(
                "matters/demo/deadlines/calendar.md",
                "| Date | Deadline / Event | Owner | Status | Source |\n|---|---|---|---|---|\n| 2027-01-15 | File status report | Team | open | order |\n",
            )
            .await
            .expect("seed deadlines");

        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

        let (status, Json(resp)) =
            matter_filing_package_handler(State(state), Path("demo".to_string()))
                .await
                .expect("filing package should be generated");

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(resp.matter_id, "demo");
        assert!(
            resp.path
                .starts_with("matters/demo/exports/filing-package-")
        );

        let exported = workspace
            .read(&resp.path)
            .await
            .expect("filing package file should exist");
        assert!(exported.content.contains("# Filing Package Index"));
        assert!(exported.content.contains("matters/demo/notes.md"));
        assert!(exported.content.contains("Template Inventory"));
    }

    #[test]
    fn list_matters_root_entries_returns_500_for_storage_errors() {
        let err = list_matters_root_entries(Err(crate::error::WorkspaceError::SearchFailed {
            reason: "boom".to_string(),
        }))
        .expect_err("search errors should map to 500");
        assert_eq!(err.0, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(err.1.contains("Search failed"));
    }

    #[test]
    fn list_matters_root_entries_allows_document_not_found_as_empty() {
        let entries =
            list_matters_root_entries(Err(crate::error::WorkspaceError::DocumentNotFound {
                doc_type: MATTER_ROOT.to_string(),
                user_id: "test-user".to_string(),
            }))
            .expect("missing matter root should be treated as empty");
        assert!(entries.is_empty());
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_invoices_list_returns_recent_limited_rows() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        for idx in 1..=3 {
            let invoice_number = format!("INV-LIST-{idx:03}");
            let params = crate::db::CreateInvoiceParams {
                matter_id: "demo".to_string(),
                invoice_number,
                status: crate::db::InvoiceStatus::Draft,
                issued_date: None,
                due_date: None,
                subtotal: rust_decimal::Decimal::ZERO,
                tax: rust_decimal::Decimal::ZERO,
                total: rust_decimal::Decimal::ZERO,
                paid_amount: rust_decimal::Decimal::ZERO,
                notes: Some("List test".to_string()),
            };
            store
                .save_invoice_draft(&state.user_id, &params, &[])
                .await
                .expect("save invoice draft");
        }

        let Json(resp) = matter_invoices_list_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterInvoicesQuery { limit: Some(2) }),
        )
        .await
        .expect("matter invoices should load");

        assert_eq!(resp.matter_id, "demo");
        assert_eq!(resp.invoices.len(), 2);
        let invoice_numbers: std::collections::HashSet<&str> = resp
            .invoices
            .iter()
            .map(|invoice| invoice.invoice_number.as_str())
            .collect();
        assert_eq!(invoice_numbers.len(), 2);
        assert!(invoice_numbers.is_subset(&std::collections::HashSet::from([
            "INV-LIST-001",
            "INV-LIST-002",
            "INV-LIST-003",
        ])));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_invoices_list_rejects_invalid_limit_values() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let err = matter_invoices_list_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterInvoicesQuery { limit: Some(0) }),
        )
        .await
        .expect_err("limit=0 should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);

        let err = matter_invoices_list_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Query(MatterInvoicesQuery { limit: Some(101) }),
        )
        .await
        .expect_err("limit above max should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_invoices_list_blocks_non_owner_access() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let other_state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: SseManager::new(),
            workspace: Some(Arc::clone(&workspace)),
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(Arc::clone(&db)),
            job_manager: None,
            prompt_queue: None,
            user_id: "other-user".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: Some(Arc::new(
                crate::channels::web::ws::WsConnectionTracker::new(),
            )),
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            chat_rate_limiter: RateLimiter::new(30, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            startup_time: std::time::Instant::now(),
            legal_config: Some(test_legal_config()),
            runtime_facts: crate::compliance::ComplianceRuntimeFacts::default(),
        });

        let err = matter_invoices_list_handler(
            State(other_state),
            Path("demo".to_string()),
            Query(MatterInvoicesQuery { limit: Some(10) }),
        )
        .await
        .expect_err("non-owner should not access matter invoices");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn invoices_finalize_marks_entries_billed_and_supports_trust_payment() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        let time_entry = store
            .create_time_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateTimeEntryParams {
                    timekeeper: "Lead".to_string(),
                    description: "Motion draft".to_string(),
                    hours: rust_decimal::Decimal::new(150, 2),
                    hourly_rate: Some(rust_decimal::Decimal::new(20000, 2)),
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 1).expect("valid date"),
                    billable: true,
                },
            )
            .await
            .expect("seed time entry");
        let expense_entry = store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Filing fee".to_string(),
                    amount: rust_decimal::Decimal::new(4000, 2),
                    category: crate::db::ExpenseCategory::FilingFee,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 1).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("seed expense entry");

        let (created_status, Json(created)) = invoices_save_handler(
            State(Arc::clone(&state)),
            Json(DraftInvoiceRequest {
                matter_id: "demo".to_string(),
                invoice_number: "INV-1001".to_string(),
                due_date: Some("2026-05-30".to_string()),
                notes: Some("Initial billing cycle".to_string()),
            }),
        )
        .await
        .expect("save draft should succeed");
        assert_eq!(created_status, StatusCode::CREATED);
        assert_eq!(created.invoice.status, "draft");
        assert_eq!(created.line_items.len(), 2);
        let invoice_id = created.invoice.id.clone();

        let Json(finalized) =
            invoices_finalize_handler(State(Arc::clone(&state)), Path(invoice_id.clone()))
                .await
                .expect("finalize should succeed");
        assert_eq!(finalized.invoice.status, "sent");

        let invoice_uuid = Uuid::parse_str(&invoice_id).expect("invoice uuid");
        let time_after = store
            .get_time_entry(&state.user_id, "demo", time_entry.id)
            .await
            .expect("get time entry")
            .expect("time entry exists");
        let expense_after = store
            .get_expense_entry(&state.user_id, "demo", expense_entry.id)
            .await
            .expect("get expense entry")
            .expect("expense entry exists");
        let invoice_id_str = invoice_uuid.to_string();
        assert_eq!(
            time_after.billed_invoice_id.as_deref(),
            Some(invoice_id_str.as_str())
        );
        assert_eq!(
            expense_after.billed_invoice_id.as_deref(),
            Some(invoice_id_str.as_str())
        );

        let (deposit_status, _deposit_body) = matter_trust_deposit_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(TrustDepositRequest {
                amount: "500.00".to_string(),
                recorded_by: "Lead".to_string(),
                description: Some("Retainer deposit".to_string()),
            }),
        )
        .await
        .expect("trust deposit should succeed");
        assert_eq!(deposit_status, StatusCode::CREATED);

        let Json(payment) = invoices_payment_handler(
            State(Arc::clone(&state)),
            Path(invoice_id.clone()),
            Json(RecordInvoicePaymentRequest {
                amount: "50.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: true,
                description: Some("Apply trust funds".to_string()),
            }),
        )
        .await
        .expect("payment should succeed");
        let paid = payment
            .invoice
            .paid_amount
            .parse::<rust_decimal::Decimal>()
            .expect("paid amount should parse");
        assert_eq!(paid, rust_decimal::Decimal::new(5000, 2));
        assert!(payment.trust_entry.is_some());

        let Json(ledger) = matter_trust_ledger_handler(State(state), Path("demo".to_string()))
            .await
            .expect("ledger should load");
        let balance = ledger
            .balance
            .parse::<rust_decimal::Decimal>()
            .expect("balance should parse");
        assert_eq!(balance, rust_decimal::Decimal::new(45000, 2));
        assert_eq!(ledger.entries.len(), 2);

        let events = crate::legal::audit::test_events_snapshot();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "invoice_finalized")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "payment_recorded")
        );
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "trust_deposit")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn invoices_payment_rejects_trust_overdraw() {
        let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
        crate::legal::audit::clear_test_events();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Service fee".to_string(),
                    amount: rust_decimal::Decimal::new(10000, 2),
                    category: crate::db::ExpenseCategory::Other,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 2).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("seed expense entry");

        let (_status, Json(created)) = invoices_save_handler(
            State(Arc::clone(&state)),
            Json(DraftInvoiceRequest {
                matter_id: "demo".to_string(),
                invoice_number: "INV-2001".to_string(),
                due_date: Some("2026-06-01".to_string()),
                notes: None,
            }),
        )
        .await
        .expect("save draft should succeed");
        let _ =
            invoices_finalize_handler(State(Arc::clone(&state)), Path(created.invoice.id.clone()))
                .await
                .expect("finalize should succeed");

        let (_deposit_status, _deposit_body) = matter_trust_deposit_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(TrustDepositRequest {
                amount: "10.00".to_string(),
                recorded_by: "Lead".to_string(),
                description: Some("Small deposit".to_string()),
            }),
        )
        .await
        .expect("trust deposit should succeed");

        let result = invoices_payment_handler(
            State(state),
            Path(created.invoice.id),
            Json(RecordInvoicePaymentRequest {
                amount: "20.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: true,
                description: Some("Attempt overdraw".to_string()),
            }),
        )
        .await;
        let err = result.expect_err("overdraw payment should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("insufficient"));
        let events = crate::legal::audit::test_events_snapshot();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "trust_withdrawal_rejected")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn invoices_payment_rejects_draft_invoice_status() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Draft-only charge".to_string(),
                    amount: rust_decimal::Decimal::new(10000, 2),
                    category: crate::db::ExpenseCategory::Other,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 6).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("seed expense entry");

        let (_status, Json(created)) = invoices_save_handler(
            State(Arc::clone(&state)),
            Json(DraftInvoiceRequest {
                matter_id: "demo".to_string(),
                invoice_number: "INV-DRAFT-100".to_string(),
                due_date: Some("2026-06-06".to_string()),
                notes: None,
            }),
        )
        .await
        .expect("save draft should succeed");

        let err = invoices_payment_handler(
            State(Arc::clone(&state)),
            Path(created.invoice.id.clone()),
            Json(RecordInvoicePaymentRequest {
                amount: "25.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: false,
                description: Some("Should fail on draft".to_string()),
            }),
        )
        .await
        .expect_err("payment on draft invoice should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("status 'draft'"));

        let invoice_after = store
            .get_invoice(
                &state.user_id,
                Uuid::parse_str(&created.invoice.id).expect("invoice uuid"),
            )
            .await
            .expect("load invoice")
            .expect("invoice exists");
        assert_eq!(invoice_after.status, crate::db::InvoiceStatus::Draft);
        assert_eq!(invoice_after.paid_amount, rust_decimal::Decimal::ZERO);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn invoices_payment_rejects_amount_above_remaining_balance() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Large service".to_string(),
                    amount: rust_decimal::Decimal::new(10000, 2),
                    category: crate::db::ExpenseCategory::Other,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 8).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("seed expense entry");

        let (_status, Json(created)) = invoices_save_handler(
            State(Arc::clone(&state)),
            Json(DraftInvoiceRequest {
                matter_id: "demo".to_string(),
                invoice_number: "INV-REM-100".to_string(),
                due_date: Some("2026-06-08".to_string()),
                notes: None,
            }),
        )
        .await
        .expect("save draft should succeed");
        let invoice_id = created.invoice.id.clone();
        let _ = invoices_finalize_handler(State(Arc::clone(&state)), Path(invoice_id.clone()))
            .await
            .expect("finalize should succeed");

        let _ = invoices_payment_handler(
            State(Arc::clone(&state)),
            Path(invoice_id.clone()),
            Json(RecordInvoicePaymentRequest {
                amount: "60.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: false,
                description: Some("Initial partial payment".to_string()),
            }),
        )
        .await
        .expect("first payment should succeed");

        let err = invoices_payment_handler(
            State(Arc::clone(&state)),
            Path(invoice_id.clone()),
            Json(RecordInvoicePaymentRequest {
                amount: "50.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: false,
                description: Some("Should exceed remaining".to_string()),
            }),
        )
        .await
        .expect_err("payment above remaining should fail");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("exceeds remaining balance"));

        let invoice_after = store
            .get_invoice(
                &state.user_id,
                Uuid::parse_str(&invoice_id).expect("invoice uuid"),
            )
            .await
            .expect("load invoice")
            .expect("invoice exists");
        assert_eq!(
            invoice_after.paid_amount,
            rust_decimal::Decimal::new(6000, 2)
        );
        assert_eq!(invoice_after.status, crate::db::InvoiceStatus::Sent);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn invoices_void_rejects_paid_invoice_status() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Payable service".to_string(),
                    amount: rust_decimal::Decimal::new(10000, 2),
                    category: crate::db::ExpenseCategory::Other,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 5, 7).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("seed expense entry");

        let (_status, Json(created)) = invoices_save_handler(
            State(Arc::clone(&state)),
            Json(DraftInvoiceRequest {
                matter_id: "demo".to_string(),
                invoice_number: "INV-PAID-100".to_string(),
                due_date: Some("2026-06-07".to_string()),
                notes: None,
            }),
        )
        .await
        .expect("save draft should succeed");

        let invoice_id = created.invoice.id.clone();
        let _ = invoices_finalize_handler(State(Arc::clone(&state)), Path(invoice_id.clone()))
            .await
            .expect("finalize should succeed");

        let _ = invoices_payment_handler(
            State(Arc::clone(&state)),
            Path(invoice_id.clone()),
            Json(RecordInvoicePaymentRequest {
                amount: "100.00".to_string(),
                recorded_by: "Lead".to_string(),
                draw_from_trust: false,
                description: Some("Mark paid".to_string()),
            }),
        )
        .await
        .expect("payment should succeed");

        let err = invoices_void_handler(State(Arc::clone(&state)), Path(invoice_id.clone()))
            .await
            .expect_err("void on paid invoice should fail");
        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("status 'paid'"));

        let invoice_after = store
            .get_invoice(
                &state.user_id,
                Uuid::parse_str(&invoice_id).expect("invoice uuid"),
            )
            .await
            .expect("load invoice")
            .expect("invoice exists");
        assert_eq!(invoice_after.status, crate::db::InvoiceStatus::Paid);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn trust_deposits_concurrently_update_balance_atomically() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist").clone();
        let user_id = state.user_id.clone();
        let barrier = Arc::new(tokio::sync::Barrier::new(3));

        let barrier_a = Arc::clone(&barrier);
        let store_a = Arc::clone(&store);
        let user_a = user_id.clone();
        let task_a = tokio::spawn(async move {
            barrier_a.wait().await;
            crate::legal::billing::record_trust_deposit(
                store_a.as_ref(),
                &user_a,
                "demo",
                rust_decimal::Decimal::new(5000, 2),
                "Lead A",
                "Concurrent deposit A",
            )
            .await
        });

        let barrier_b = Arc::clone(&barrier);
        let store_b = Arc::clone(&store);
        let user_b = user_id.clone();
        let task_b = tokio::spawn(async move {
            barrier_b.wait().await;
            crate::legal::billing::record_trust_deposit(
                store_b.as_ref(),
                &user_b,
                "demo",
                rust_decimal::Decimal::new(5000, 2),
                "Lead B",
                "Concurrent deposit B",
            )
            .await
        });

        barrier.wait().await;
        let entry_a = task_a
            .await
            .expect("task A should join")
            .expect("deposit A should succeed");
        let entry_b = task_b
            .await
            .expect("task B should join")
            .expect("deposit B should succeed");

        let mut balances = vec![entry_a.balance_after, entry_b.balance_after];
        balances.sort();
        assert_eq!(
            balances,
            vec![
                rust_decimal::Decimal::new(5000, 2),
                rust_decimal::Decimal::new(10000, 2)
            ]
        );

        let balance = store
            .current_trust_balance(&state.user_id, "demo")
            .await
            .expect("read balance");
        assert_eq!(balance, rust_decimal::Decimal::new(10000, 2));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_time_create_rejects_non_positive_hours() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let result = matter_time_create_handler(
            State(state),
            Path("demo".to_string()),
            Json(CreateTimeEntryRequest {
                timekeeper: "Paralegal".to_string(),
                description: "Prepare draft".to_string(),
                hours: "0".to_string(),
                hourly_rate: Some("200".to_string()),
                entry_date: "2026-04-10".to_string(),
                billable: Some(true),
            }),
        )
        .await;

        let err = result.expect_err("zero-hour time entry should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("'hours' must be greater than 0"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_expense_create_rejects_non_positive_amount() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let result = matter_expenses_create_handler(
            State(state),
            Path("demo".to_string()),
            Json(CreateExpenseEntryRequest {
                submitted_by: "Associate".to_string(),
                description: "Filing fee".to_string(),
                amount: "0".to_string(),
                category: "filing_fee".to_string(),
                entry_date: "2026-04-10".to_string(),
                receipt_path: None,
                billable: Some(true),
            }),
        )
        .await;

        let err = result.expect_err("zero-amount expense entry should be rejected");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("'amount' must be greater than 0"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_time_delete_rejects_billed_entry() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        let billed_entry = store
            .create_time_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateTimeEntryParams {
                    timekeeper: "Lead".to_string(),
                    description: "Billed work".to_string(),
                    hours: rust_decimal::Decimal::new(150, 2),
                    hourly_rate: Some(rust_decimal::Decimal::new(30000, 2)),
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 9).expect("valid date"),
                    billable: true,
                },
            )
            .await
            .expect("create billed seed entry");
        let unbilled_entry = store
            .create_time_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateTimeEntryParams {
                    timekeeper: "Lead".to_string(),
                    description: "Unbilled work".to_string(),
                    hours: rust_decimal::Decimal::new(50, 2),
                    hourly_rate: Some(rust_decimal::Decimal::new(30000, 2)),
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 10).expect("valid date"),
                    billable: true,
                },
            )
            .await
            .expect("create unbilled seed entry");

        let marked = store
            .mark_time_entries_billed(&state.user_id, &[billed_entry.id], "inv-1001")
            .await
            .expect("mark billed entry");
        assert_eq!(marked, 1);

        let billed_after = store
            .get_time_entry(&state.user_id, "demo", billed_entry.id)
            .await
            .expect("load billed entry")
            .expect("billed entry should exist");
        let unbilled_after = store
            .get_time_entry(&state.user_id, "demo", unbilled_entry.id)
            .await
            .expect("load unbilled entry")
            .expect("unbilled entry should exist");
        assert_eq!(billed_after.billed_invoice_id.as_deref(), Some("inv-1001"));
        assert!(unbilled_after.billed_invoice_id.is_none());

        let billed_delete = matter_time_delete_handler(
            State(Arc::clone(&state)),
            Path(("demo".to_string(), billed_entry.id.to_string())),
        )
        .await;
        let err = billed_delete.expect_err("billed entry should not be deletable");
        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("billed"));

        let unbilled_delete = matter_time_delete_handler(
            State(state),
            Path(("demo".to_string(), unbilled_entry.id.to_string())),
        )
        .await
        .expect("unbilled entry should be deletable");
        assert_eq!(unbilled_delete, StatusCode::NO_CONTENT);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_time_summary_aggregates_hours_and_expenses() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");
        let store = state.store.as_ref().expect("store should exist");

        let time_one = store
            .create_time_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateTimeEntryParams {
                    timekeeper: "Lead".to_string(),
                    description: "Billable review".to_string(),
                    hours: rust_decimal::Decimal::new(150, 2),
                    hourly_rate: Some(rust_decimal::Decimal::new(35000, 2)),
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 11).expect("valid date"),
                    billable: true,
                },
            )
            .await
            .expect("create first time entry");
        store
            .create_time_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateTimeEntryParams {
                    timekeeper: "Paralegal".to_string(),
                    description: "Internal prep".to_string(),
                    hours: rust_decimal::Decimal::new(50, 2),
                    hourly_rate: None,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 11).expect("valid date"),
                    billable: false,
                },
            )
            .await
            .expect("create second time entry");
        let expense_one = store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Court filing fee".to_string(),
                    amount: rust_decimal::Decimal::new(10000, 2),
                    category: crate::db::ExpenseCategory::FilingFee,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 11).expect("valid date"),
                    receipt_path: None,
                    billable: true,
                },
            )
            .await
            .expect("create first expense entry");
        store
            .create_expense_entry(
                &state.user_id,
                "demo",
                &crate::db::CreateExpenseEntryParams {
                    submitted_by: "Lead".to_string(),
                    description: "Internal courier".to_string(),
                    amount: rust_decimal::Decimal::new(4000, 2),
                    category: crate::db::ExpenseCategory::Other,
                    entry_date: chrono::NaiveDate::from_ymd_opt(2026, 4, 11).expect("valid date"),
                    receipt_path: None,
                    billable: false,
                },
            )
            .await
            .expect("create second expense entry");
        store
            .mark_time_entries_billed(&state.user_id, &[time_one.id], "inv-2001")
            .await
            .expect("mark one time entry billed");
        store
            .mark_expense_entries_billed(&state.user_id, &[expense_one.id], "inv-2001")
            .await
            .expect("mark one expense entry billed");

        let Json(summary) = matter_time_summary_handler(State(state), Path("demo".to_string()))
            .await
            .expect("summary handler should succeed");

        let total_hours = summary
            .total_hours
            .parse::<rust_decimal::Decimal>()
            .expect("parse total hours");
        let billable_hours = summary
            .billable_hours
            .parse::<rust_decimal::Decimal>()
            .expect("parse billable hours");
        let unbilled_hours = summary
            .unbilled_hours
            .parse::<rust_decimal::Decimal>()
            .expect("parse unbilled hours");
        let total_expenses = summary
            .total_expenses
            .parse::<rust_decimal::Decimal>()
            .expect("parse total expenses");
        let billable_expenses = summary
            .billable_expenses
            .parse::<rust_decimal::Decimal>()
            .expect("parse billable expenses");
        let unbilled_expenses = summary
            .unbilled_expenses
            .parse::<rust_decimal::Decimal>()
            .expect("parse unbilled expenses");

        assert_eq!(total_hours, rust_decimal::Decimal::new(200, 2));
        assert_eq!(billable_hours, rust_decimal::Decimal::new(150, 2));
        assert_eq!(unbilled_hours, rust_decimal::Decimal::new(50, 2));
        assert_eq!(total_expenses, rust_decimal::Decimal::new(14000, 2));
        assert_eq!(billable_expenses, rust_decimal::Decimal::new(10000, 2));
        assert_eq!(unbilled_expenses, rust_decimal::Decimal::new(4000, 2));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matter_detail_work_and_finance_endpoints_return_expected_data() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        seed_valid_matter(workspace.as_ref(), "demo").await;
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));
        ensure_matter_db_row_from_workspace(state.as_ref(), "demo")
            .await
            .expect("sync matter row");

        let _ = matter_tasks_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateMatterTaskRequest {
                title: "Draft chronology".to_string(),
                description: Some("Capture filing timeline".to_string()),
                status: Some("todo".to_string()),
                assignee: Some("Paralegal".to_string()),
                due_at: None,
                blocked_by: Vec::new(),
            }),
        )
        .await
        .expect("create task");

        let _ = matter_notes_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateMatterNoteRequest {
                author: "Lead".to_string(),
                body: "Initial intake complete".to_string(),
                pinned: true,
            }),
        )
        .await
        .expect("create note");

        let _ = matter_time_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateTimeEntryRequest {
                timekeeper: "Lead".to_string(),
                description: "Case strategy review".to_string(),
                hours: "1.25".to_string(),
                hourly_rate: Some("300".to_string()),
                entry_date: "2026-04-12".to_string(),
                billable: Some(true),
            }),
        )
        .await
        .expect("create time entry");

        let _ = matter_expenses_create_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(CreateExpenseEntryRequest {
                submitted_by: "Lead".to_string(),
                description: "Filing courier".to_string(),
                amount: "45.00".to_string(),
                category: "other".to_string(),
                entry_date: "2026-04-12".to_string(),
                receipt_path: None,
                billable: Some(true),
            }),
        )
        .await
        .expect("create expense entry");

        let _ = matter_trust_deposit_handler(
            State(Arc::clone(&state)),
            Path("demo".to_string()),
            Json(TrustDepositRequest {
                amount: "500.00".to_string(),
                recorded_by: "Lead".to_string(),
                description: Some("Initial retainer".to_string()),
            }),
        )
        .await
        .expect("create trust deposit");

        let Json(tasks) = matter_tasks_list_handler(State(Arc::clone(&state)), Path("demo".into()))
            .await
            .expect("list tasks");
        assert_eq!(tasks.tasks.len(), 1);

        let Json(notes) = matter_notes_list_handler(State(Arc::clone(&state)), Path("demo".into()))
            .await
            .expect("list notes");
        assert_eq!(notes.notes.len(), 1);

        let Json(time_entries) =
            matter_time_list_handler(State(Arc::clone(&state)), Path("demo".into()))
                .await
                .expect("list time entries");
        assert_eq!(time_entries.entries.len(), 1);

        let Json(expense_entries) =
            matter_expenses_list_handler(State(Arc::clone(&state)), Path("demo".into()))
                .await
                .expect("list expense entries");
        assert_eq!(expense_entries.entries.len(), 1);

        let Json(summary) =
            matter_time_summary_handler(State(Arc::clone(&state)), Path("demo".into()))
                .await
                .expect("time summary");
        let total_hours = summary
            .total_hours
            .parse::<rust_decimal::Decimal>()
            .expect("hours decimal");
        assert_eq!(total_hours, rust_decimal::Decimal::new(125, 2));

        let Json(ledger) = matter_trust_ledger_handler(State(state), Path("demo".into()))
            .await
            .expect("trust ledger");
        assert_eq!(ledger.matter_id, "demo");
        assert_eq!(ledger.entries.len(), 1);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_write_handler_invalidates_conflict_cache() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let mut legal = test_legal_config();
        legal.require_matter_context = false;
        let state = test_gateway_state_with_store_workspace_and_legal(
            Arc::clone(&db),
            Arc::clone(&workspace),
            legal,
        );

        workspace
            .write(
                "conflicts.json",
                r#"[{"name":"Alpha Holdings","aliases":["Alpha"]}]"#,
            )
            .await
            .expect("seed conflicts");

        let mut legal = crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve");
        legal.active_matter = None;
        legal.enabled = true;
        legal.conflict_check_enabled = true;

        let first =
            crate::legal::matter::detect_conflict(workspace.as_ref(), &legal, "Alpha Holdings")
                .await;
        assert_eq!(first.as_deref(), Some("Alpha Holdings"));
        assert_eq!(
            crate::legal::matter::conflict_cache_refresh_count_for_tests(),
            1
        );

        let write_result = memory_write_handler(
            State(state),
            Json(MemoryWriteRequest {
                path: "conflicts.json".to_string(),
                content: r#"[{"name":"Beta Partners","aliases":["Beta"]}]"#.to_string(),
            }),
        )
        .await
        .expect("memory write should succeed");
        assert_eq!(write_result.path, "conflicts.json");

        let second =
            crate::legal::matter::detect_conflict(workspace.as_ref(), &legal, "Beta Partners")
                .await;
        assert_eq!(second.as_deref(), Some("Beta Partners"));
        assert_eq!(
            crate::legal::matter::conflict_cache_refresh_count_for_tests(),
            2
        );
    }
}
