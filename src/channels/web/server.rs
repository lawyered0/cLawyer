//! Axum HTTP server for the web gateway.
//!
//! Handles all API routes: chat, memory, jobs, health, and static file serving.

use std::convert::Infallible;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, Query, State, WebSocketUpgrade},
    http::{StatusCode, header},
    middleware,
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::StreamExt;
use tower_http::cors::{AllowHeaders, CorsLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use uuid::Uuid;

use crate::agent::SessionManager;
use crate::channels::IncomingMessage;
use crate::channels::web::auth::{AuthState, auth_middleware};
use crate::channels::web::handlers::skills::{
    skills_install_handler, skills_list_handler, skills_remove_handler, skills_search_handler,
};
use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::web::sse::SseManager;
use crate::channels::web::types::*;
use crate::db::Database;
use crate::extensions::ExtensionManager;
use crate::orchestrator::job_manager::ContainerJobManager;
use crate::tools::ToolRegistry;
use crate::workspace::Workspace;

/// Shared prompt queue: maps job IDs to pending follow-up prompts for Claude Code bridges.
pub type PromptQueue = Arc<
    tokio::sync::Mutex<
        std::collections::HashMap<
            uuid::Uuid,
            std::collections::VecDeque<crate::orchestrator::api::PendingPrompt>,
        >,
    >,
>;

/// Simple sliding-window rate limiter.
///
/// Tracks the number of requests in the current window. Resets when the window expires.
/// Not per-IP (since this is a single-user gateway with auth), but prevents flooding.
pub struct RateLimiter {
    /// Requests remaining in the current window.
    remaining: AtomicU64,
    /// Epoch second when the current window started.
    window_start: AtomicU64,
    /// Maximum requests per window.
    max_requests: u64,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u64, window_secs: u64) -> Self {
        Self {
            remaining: AtomicU64::new(max_requests),
            window_start: AtomicU64::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            ),
            max_requests,
            window_secs,
        }
    }

    /// Try to consume one request. Returns `true` if allowed, `false` if rate limited.
    pub fn check(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let window = self.window_start.load(Ordering::Relaxed);
        if now.saturating_sub(window) >= self.window_secs {
            // Window expired, reset
            self.window_start.store(now, Ordering::Relaxed);
            self.remaining
                .store(self.max_requests - 1, Ordering::Relaxed);
            return true;
        }

        // Try to decrement remaining
        loop {
            let current = self.remaining.load(Ordering::Relaxed);
            if current == 0 {
                return false;
            }
            if self
                .remaining
                .compare_exchange_weak(current, current - 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }
}

/// Shared state for all gateway handlers.
pub struct GatewayState {
    /// Channel to send messages to the agent loop.
    pub msg_tx: tokio::sync::RwLock<Option<mpsc::Sender<IncomingMessage>>>,
    /// SSE broadcast manager.
    pub sse: SseManager,
    /// Workspace for memory API.
    pub workspace: Option<Arc<Workspace>>,
    /// Session manager for thread info.
    pub session_manager: Option<Arc<SessionManager>>,
    /// Log broadcaster for the logs SSE endpoint.
    pub log_broadcaster: Option<Arc<LogBroadcaster>>,
    /// Handle for changing the tracing log level at runtime.
    pub log_level_handle: Option<Arc<crate::channels::web::log_layer::LogLevelHandle>>,
    /// Extension manager for extension management API.
    pub extension_manager: Option<Arc<ExtensionManager>>,
    /// Tool registry for listing registered tools.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Database store for sandbox job persistence.
    pub store: Option<Arc<dyn Database>>,
    /// Container job manager for sandbox operations.
    pub job_manager: Option<Arc<ContainerJobManager>>,
    /// Prompt queue for Claude Code follow-up prompts.
    pub prompt_queue: Option<PromptQueue>,
    /// User ID for this gateway.
    pub user_id: String,
    /// Shutdown signal sender.
    pub shutdown_tx: tokio::sync::RwLock<Option<oneshot::Sender<()>>>,
    /// WebSocket connection tracker.
    pub ws_tracker: Option<Arc<crate::channels::web::ws::WsConnectionTracker>>,
    /// LLM provider for OpenAI-compatible API proxy.
    pub llm_provider: Option<Arc<dyn crate::llm::LlmProvider>>,
    /// Skill registry for skill management API.
    pub skill_registry: Option<Arc<std::sync::RwLock<crate::skills::SkillRegistry>>>,
    /// Skill catalog for searching the ClawHub registry.
    pub skill_catalog: Option<Arc<crate::skills::catalog::SkillCatalog>>,
    /// Rate limiter for chat endpoints (30 messages per 60 seconds).
    pub chat_rate_limiter: RateLimiter,
    /// Registry catalog entries for the available extensions API.
    /// Populated at startup from `registry/` manifests, independent of extension manager.
    pub registry_entries: Vec<crate::extensions::RegistryEntry>,
    /// Cost guard for token/cost tracking.
    pub cost_guard: Option<Arc<crate::agent::cost_guard::CostGuard>>,
    /// Server startup time for uptime calculation.
    pub startup_time: std::time::Instant,
    /// Legal config for legal-policy-aware web endpoints.
    pub legal_config: Option<crate::config::LegalConfig>,
}

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
    let public = Router::new().route("/api/health", get(health_handler));

    // Protected routes (require auth)
    let auth_state = AuthState { token: auth_token };
    let protected = Router::new()
        // Chat
        .route("/api/chat/send", post(chat_send_handler))
        .route("/api/chat/approval", post(chat_approval_handler))
        .route("/api/chat/auth-token", post(chat_auth_token_handler))
        .route("/api/chat/auth-cancel", post(chat_auth_cancel_handler))
        .route("/api/chat/events", get(chat_events_handler))
        .route("/api/chat/ws", get(chat_ws_handler))
        .route("/api/chat/history", get(chat_history_handler))
        .route("/api/chat/threads", get(chat_threads_handler))
        .route("/api/chat/thread/new", post(chat_new_thread_handler))
        // Memory
        .route("/api/memory/tree", get(memory_tree_handler))
        .route("/api/memory/list", get(memory_list_handler))
        .route("/api/memory/read", get(memory_read_handler))
        .route("/api/memory/write", post(memory_write_handler))
        .route("/api/memory/search", post(memory_search_handler))
        .route(
            "/api/memory/upload",
            post(memory_upload_handler).layer(DefaultBodyLimit::max(UPLOAD_FILE_SIZE_LIMIT)),
        )
        // Matters
        .route(
            "/api/matters",
            get(matters_list_handler).post(matters_create_handler),
        )
        .route(
            "/api/matters/active",
            get(matters_active_get_handler).post(matters_active_set_handler),
        )
        .route("/api/matters/{id}/documents", get(matter_documents_handler))
        .route("/api/matters/{id}/dashboard", get(matter_dashboard_handler))
        .route("/api/matters/{id}/deadlines", get(matter_deadlines_handler))
        .route("/api/matters/{id}/templates", get(matter_templates_handler))
        .route(
            "/api/matters/{id}/templates/apply",
            post(matter_template_apply_handler),
        )
        .route(
            "/api/matters/{id}/filing-package",
            post(matter_filing_package_handler),
        )
        .route(
            "/api/matters/conflicts/check",
            post(matters_conflicts_check_handler),
        )
        .route("/api/legal/audit", get(legal_audit_list_handler))
        // Jobs
        .route("/api/jobs", get(jobs_list_handler))
        .route("/api/jobs/summary", get(jobs_summary_handler))
        .route("/api/jobs/{id}", get(jobs_detail_handler))
        .route("/api/jobs/{id}/cancel", post(jobs_cancel_handler))
        .route("/api/jobs/{id}/restart", post(jobs_restart_handler))
        .route("/api/jobs/{id}/prompt", post(jobs_prompt_handler))
        .route("/api/jobs/{id}/events", get(jobs_events_handler))
        .route("/api/jobs/{id}/files/list", get(job_files_list_handler))
        .route("/api/jobs/{id}/files/read", get(job_files_read_handler))
        // Logs
        .route("/api/logs/events", get(logs_events_handler))
        .route("/api/logs/level", get(logs_level_get_handler))
        .route(
            "/api/logs/level",
            axum::routing::put(logs_level_set_handler),
        )
        // Extensions
        .route("/api/extensions", get(extensions_list_handler))
        .route("/api/extensions/tools", get(extensions_tools_handler))
        .route("/api/extensions/registry", get(extensions_registry_handler))
        .route("/api/extensions/install", post(extensions_install_handler))
        .route(
            "/api/extensions/{name}/activate",
            post(extensions_activate_handler),
        )
        .route(
            "/api/extensions/{name}/remove",
            post(extensions_remove_handler),
        )
        .route(
            "/api/extensions/{name}/setup",
            get(extensions_setup_handler).post(extensions_setup_submit_handler),
        )
        // Pairing
        .route("/api/pairing/{channel}", get(pairing_list_handler))
        .route(
            "/api/pairing/{channel}/approve",
            post(pairing_approve_handler),
        )
        // Routines
        .route(
            "/api/routines",
            get(routines_list_handler).post(routines_create_handler),
        )
        .route("/api/routines/summary", get(routines_summary_handler))
        .route("/api/routines/{id}", get(routines_detail_handler))
        .route("/api/routines/{id}/trigger", post(routines_trigger_handler))
        .route("/api/routines/{id}/toggle", post(routines_toggle_handler))
        .route(
            "/api/routines/{id}",
            axum::routing::delete(routines_delete_handler),
        )
        .route("/api/routines/{id}/runs", get(routines_runs_handler))
        // Skills
        .route("/api/skills", get(skills_list_handler))
        .route("/api/skills/search", post(skills_search_handler))
        .route("/api/skills/install", post(skills_install_handler))
        .route(
            "/api/skills/{name}",
            axum::routing::delete(skills_remove_handler),
        )
        // Settings
        .route("/api/settings", get(settings_list_handler))
        .route("/api/settings/export", get(settings_export_handler))
        .route("/api/settings/import", post(settings_import_handler))
        .route("/api/settings/{key}", get(settings_get_handler))
        .route(
            "/api/settings/{key}",
            axum::routing::put(settings_set_handler),
        )
        .route(
            "/api/settings/{key}",
            axum::routing::delete(settings_delete_handler),
        )
        // Gateway control plane
        .route("/api/gateway/status", get(gateway_status_handler))
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
    let statics = Router::new()
        .route("/", get(index_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/favicon.ico", get(favicon_handler));

    // Project file serving (behind auth to prevent unauthorized file access).
    let projects = Router::new()
        .route("/projects/{project_id}", get(project_redirect_handler))
        .route("/projects/{project_id}/", get(project_index_handler))
        .route("/projects/{project_id}/{*path}", get(project_file_handler))
        .route_layer(middleware::from_fn_with_state(
            auth_state.clone(),
            auth_middleware,
        ));

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

async fn index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/index.html"),
    )
}

async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/style.css"),
    )
}

async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("static/app.js"),
    )
}

async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("static/favicon.ico").as_slice(),
    )
}

// --- Health ---

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

// --- Chat handlers ---

async fn load_active_matter_for_chat(state: &GatewayState) -> Option<String> {
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
    let sanitized = crate::legal::policy::sanitize_matter_id(raw);
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

async fn build_chat_message_metadata(
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

async fn chat_send_handler(
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
    msg = msg
        .with_metadata(build_chat_message_metadata(state.as_ref(), req.thread_id.as_deref()).await);

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

async fn chat_approval_handler(
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

    // Build a structured ExecApproval submission as JSON, sent through the
    // existing message pipeline so the agent loop picks it up.
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
    msg = msg
        .with_metadata(build_chat_message_metadata(state.as_ref(), req.thread_id.as_deref()).await);

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

/// Submit an auth token directly to the extension manager, bypassing the message pipeline.
///
/// The token never touches the LLM, chat history, or SSE stream.
async fn chat_auth_token_handler(
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
        // Auto-activate so tools are available immediately
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

        // Clear auth mode on the active thread
        clear_auth_mode(&state).await;

        state.sse.broadcast(SseEvent::AuthCompleted {
            extension_name: req.extension_name,
            success: true,
            message: msg.clone(),
        });

        Ok(Json(ActionResponse::ok(msg)))
    } else {
        // Re-emit auth_required for retry
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

/// Cancel an in-progress auth flow.
async fn chat_auth_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Json(_req): Json<AuthCancelRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    clear_auth_mode(&state).await;
    Ok(Json(ActionResponse::ok("Auth cancelled")))
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

async fn chat_events_handler(
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

async fn chat_ws_handler(
    headers: axum::http::HeaderMap,
    ws: WebSocketUpgrade,
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate Origin header to prevent cross-site WebSocket hijacking.
    // Require the header outright; browsers always send it for WS upgrades,
    // so a missing Origin means a non-browser client trying to bypass the check.
    let origin = headers
        .get("origin")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            (
                StatusCode::FORBIDDEN,
                "WebSocket Origin header required".to_string(),
            )
        })?;

    // Extract the host from the origin and compare exactly, so that
    // crafted origins like "http://localhost.evil.com" are rejected.
    // Origin format is "scheme://host[:port]".
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

#[derive(Deserialize)]
struct HistoryQuery {
    thread_id: Option<String>,
    limit: Option<usize>,
    before: Option<String>,
}

async fn chat_history_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<HistoryResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&state.user_id).await;
    let sess = session.lock().await;

    let limit = query.limit.unwrap_or(50);
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

    // Find the thread
    let thread_id = if let Some(ref tid) = query.thread_id {
        Uuid::parse_str(tid)
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid thread_id".to_string()))?
    } else {
        sess.active_thread
            .ok_or((StatusCode::NOT_FOUND, "No active thread".to_string()))?
    };

    // Verify the thread belongs to the authenticated user before returning any data.
    // In-memory threads are already scoped by user via session_manager, but DB
    // lookups could expose another user's conversation if the UUID is guessed.
    if query.thread_id.is_some()
        && let Some(ref store) = state.store
    {
        let owned = store
            .conversation_belongs_to_user(thread_id, &state.user_id)
            .await
            .unwrap_or(false);
        if !owned && !sess.threads.contains_key(&thread_id) {
            return Err((StatusCode::NOT_FOUND, "Thread not found".to_string()));
        }
    }

    // For paginated requests (before cursor set), always go to DB
    if before_cursor.is_some()
        && let Some(ref store) = state.store
    {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, before_cursor, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
        let turns = build_turns_from_db_messages(&messages);
        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more,
            oldest_timestamp,
        }));
    }

    // Try in-memory first (freshest data for active threads)
    if let Some(thread) = sess.threads.get(&thread_id)
        && !thread.turns.is_empty()
    {
        let turns: Vec<TurnInfo> = thread
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
            .collect();

        return Ok(Json(HistoryResponse {
            thread_id,
            turns,
            has_more: false,
            oldest_timestamp: None,
        }));
    }

    // Fall back to DB for historical threads not in memory (paginated)
    if let Some(ref store) = state.store {
        let (messages, has_more) = store
            .list_conversation_messages_paginated(thread_id, None, limit as i64)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if !messages.is_empty() {
            let oldest_timestamp = messages.first().map(|m| m.created_at.to_rfc3339());
            let turns = build_turns_from_db_messages(&messages);
            return Ok(Json(HistoryResponse {
                thread_id,
                turns,
                has_more,
                oldest_timestamp,
            }));
        }
    }

    // Empty thread (just created, no messages yet)
    Ok(Json(HistoryResponse {
        thread_id,
        turns: Vec::new(),
        has_more: false,
        oldest_timestamp: None,
    }))
}

/// Build TurnInfo pairs from flat DB messages (alternating user/assistant).
fn build_turns_from_db_messages(messages: &[crate::history::ConversationMessage]) -> Vec<TurnInfo> {
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

async fn chat_threads_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ThreadListResponse>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&state.user_id).await;
    let sess = session.lock().await;

    // Try DB first for persistent thread list
    if let Some(ref store) = state.store {
        // Auto-create assistant thread if it doesn't exist
        let assistant_id = store
            .get_or_create_assistant_conversation(&state.user_id, "gateway")
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        if let Ok(summaries) = store
            .list_conversations_with_preview(&state.user_id, "gateway", 50)
            .await
        {
            let mut assistant_thread = None;
            let mut threads = Vec::new();

            for s in &summaries {
                let info = ThreadInfo {
                    id: s.id,
                    state: "Idle".to_string(),
                    turn_count: (s.message_count / 2).max(0) as usize,
                    created_at: s.started_at.to_rfc3339(),
                    updated_at: s.last_activity.to_rfc3339(),
                    title: s.title.clone(),
                    thread_type: s.thread_type.clone(),
                };

                if s.id == assistant_id {
                    assistant_thread = Some(info);
                } else {
                    threads.push(info);
                }
            }

            // If assistant wasn't in the list (0 messages), synthesize it
            if assistant_thread.is_none() {
                assistant_thread = Some(ThreadInfo {
                    id: assistant_id,
                    state: "Idle".to_string(),
                    turn_count: 0,
                    created_at: chrono::Utc::now().to_rfc3339(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    title: None,
                    thread_type: Some("assistant".to_string()),
                });
            }

            return Ok(Json(ThreadListResponse {
                assistant_thread,
                threads,
                active_thread: sess.active_thread,
            }));
        }
    }

    // Fallback: in-memory only (no assistant thread without DB)
    let threads: Vec<ThreadInfo> = sess
        .threads
        .values()
        .map(|t| ThreadInfo {
            id: t.id,
            state: format!("{:?}", t.state),
            turn_count: t.turns.len(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            title: None,
            thread_type: None,
        })
        .collect();

    Ok(Json(ThreadListResponse {
        assistant_thread: None,
        threads,
        active_thread: sess.active_thread,
    }))
}

async fn chat_new_thread_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ThreadInfo>, (StatusCode, String)> {
    let session_manager = state.session_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Session manager not available".to_string(),
    ))?;

    let session = session_manager.get_or_create_session(&state.user_id).await;
    let mut sess = session.lock().await;
    let thread = sess.create_thread();
    let thread_id = thread.id;
    let info = ThreadInfo {
        id: thread.id,
        state: format!("{:?}", thread.state),
        turn_count: thread.turns.len(),
        created_at: thread.created_at.to_rfc3339(),
        updated_at: thread.updated_at.to_rfc3339(),
        title: None,
        thread_type: Some("thread".to_string()),
    };

    // Persist the empty conversation row with thread_type metadata
    if let Some(ref store) = state.store {
        let store = Arc::clone(store);
        let user_id = state.user_id.clone();
        tokio::spawn(async move {
            if let Err(e) = store
                .ensure_conversation(thread_id, "gateway", &user_id, None)
                .await
            {
                tracing::warn!("Failed to persist new thread: {}", e);
            }
            let metadata_val = serde_json::json!("thread");
            if let Err(e) = store
                .update_conversation_metadata_field(thread_id, "thread_type", &metadata_val)
                .await
            {
                tracing::warn!("Failed to set thread_type metadata: {}", e);
            }
        });
    }

    Ok(Json(info))
}

// --- Memory handlers ---

#[derive(Deserialize)]
struct TreeQuery {
    #[allow(dead_code)]
    depth: Option<usize>,
}

async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    // Build tree from list_all (flat list of all paths)
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect unique directories and files
    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        // Add parent directories
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        // Add the file itself
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(MemoryTreeResponse { entries }))
}

#[derive(Deserialize)]
struct ListQuery {
    path: Option<String>,
}

async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

#[derive(Deserialize)]
struct ReadQuery {
    path: String,
}

async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    workspace
        .write(&req.path, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if crate::legal::matter::is_workspace_conflicts_path(&req.path) {
        crate::legal::matter::invalidate_conflict_cache();
    }

    Ok(Json(MemoryWriteResponse {
        path: req.path,
        status: "written",
    }))
}

async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.document_id.to_string(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(Json(MemorySearchResponse { results: hits }))
}

/// Maximum size accepted for a single uploaded file (10 MiB).
const UPLOAD_FILE_SIZE_LIMIT: usize = 10 * 1024 * 1024;

async fn memory_upload_handler(
    State(state): State<Arc<GatewayState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<MemoryUploadResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let mut uploaded: Vec<UploadedFile> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Multipart read error: {e}"),
        )
    })? {
        // Derive a safe filename: take the basename only, keep alphanumerics
        // plus a small allow-set of punctuation, collapse empty result to a
        // safe default.  This prevents path traversal via the filename header.
        let raw_name = field.file_name().unwrap_or("document.txt").to_string();
        let safe_name: String = raw_name
            .rsplit('/')
            .next()
            .unwrap_or("document.txt")
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
            .collect();
        let safe_name = if safe_name.trim().is_empty() {
            "document.txt".to_string()
        } else {
            safe_name.trim().to_string()
        };
        let dest_path = format!("uploads/{safe_name}");

        // Read the field body, enforcing the per-file size limit.
        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to read upload body: {e}"),
            )
        })?;

        if data.len() > UPLOAD_FILE_SIZE_LIMIT {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "File '{}' exceeds the 10 MiB upload limit ({} bytes)",
                    raw_name,
                    data.len()
                ),
            ));
        }

        // Workspace stores text; reject binary (non-UTF-8) content with a
        // helpful error rather than storing garbled data.
        let content = String::from_utf8(data.to_vec()).map_err(|_| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "File '{}' contains non-UTF-8 bytes. Only plain-text files are supported.",
                    raw_name
                ),
            )
        })?;

        let byte_count = content.len();
        workspace
            .write(&dest_path, &content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        uploaded.push(UploadedFile {
            path: dest_path,
            bytes: byte_count,
            status: "written",
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(MemoryUploadResponse { files: uploaded }),
    ))
}

// --- Matter handlers ---

/// The workspace path prefix where matter directories live.
const MATTER_ROOT: &str = "matters";
/// Settings key used to persist the active matter ID.
const MATTER_ACTIVE_SETTING: &str = "legal.active_matter";
/// Maximum number of audit log lines scanned per request.
const MAX_AUDIT_SCAN_LINES: usize = 10_000;

fn legal_config_for_gateway(state: &GatewayState) -> crate::config::LegalConfig {
    state.legal_config.clone().unwrap_or_else(|| {
        crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve")
    })
}

#[derive(Debug, Default, Deserialize)]
struct LegalAuditQuery {
    limit: Option<usize>,
    offset: Option<usize>,
    event_type: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LegalAuditEventLine {
    ts: String,
    event_type: String,
    details: serde_json::Value,
    metrics: serde_json::Value,
    #[serde(default)]
    prev_hash: Option<String>,
    #[serde(default)]
    hash: Option<String>,
}

fn parse_utc_query_ts(
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

#[derive(Debug, Default, Deserialize)]
struct MatterDocumentsQuery {
    include_templates: Option<bool>,
}

fn sanitize_matter_id_for_route(raw: &str) -> Result<String, (StatusCode, String)> {
    let sanitized = crate::legal::policy::sanitize_matter_id(raw);
    if sanitized.is_empty() {
        return Err((StatusCode::NOT_FOUND, "Matter not found".to_string()));
    }
    Ok(sanitized)
}

async fn ensure_existing_matter_for_route(
    workspace: &Workspace,
    raw_matter_id: &str,
) -> Result<String, (StatusCode, String)> {
    let matter_id = sanitize_matter_id_for_route(raw_matter_id)?;
    match crate::legal::matter::read_matter_metadata_for_root(workspace, MATTER_ROOT, &matter_id)
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

fn parse_template_name(raw: &str) -> Result<String, (StatusCode, String)> {
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

async fn choose_template_apply_destination(
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

async fn list_matter_documents_recursive(
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
                name,
                path: entry.path.clone(),
                is_dir: entry.is_directory,
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

fn checklist_completion_from_markdown(markdown: &str) -> (usize, usize) {
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

async fn list_matter_templates(
    workspace: &Workspace,
    matter_id: &str,
) -> Result<Vec<MatterTemplateInfo>, (StatusCode, String)> {
    let templates_path = format!("{MATTER_ROOT}/{matter_id}/templates");
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
                name,
                path: entry.path,
                updated_at: entry.updated_at.map(|dt| dt.to_rfc3339()),
            })
        })
        .collect();
    templates.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(templates)
}

async fn choose_filing_package_destination(
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

async fn matters_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<MattersListResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    // List top-level entries under the matter root. Treat a missing
    // directory as an empty list rather than an error — the matters/ dir
    // is seeded lazily on first use.
    let entries = list_matters_root_entries(workspace.list(MATTER_ROOT).await)?;

    let mut matters: Vec<MatterInfo> = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        let dir_name = entry.path.rsplit('/').next().unwrap_or("").to_string();
        // Skip the template scaffold directory written by seed_legal_workspace.
        if dir_name.is_empty() || dir_name == "_template" {
            continue;
        }

        // Read matter.yaml for rich metadata; fall back to ID-only if missing
        // or malformed (e.g. the matter was created manually without a YAML).
        let meta: Option<crate::legal::matter::MatterMetadata> = {
            let yaml_path = format!("{MATTER_ROOT}/{dir_name}/matter.yaml");
            workspace
                .read(&yaml_path)
                .await
                .ok()
                .and_then(|doc| serde_yml::from_str(&doc.content).ok())
        };

        matters.push(MatterInfo {
            id: dir_name,
            client: meta.as_ref().map(|m| m.client.clone()),
            confidentiality: meta.as_ref().map(|m| m.confidentiality.clone()),
            team: meta.as_ref().map(|m| m.team.clone()).unwrap_or_default(),
            adversaries: meta
                .as_ref()
                .map(|m| m.adversaries.clone())
                .unwrap_or_default(),
            retention: meta.as_ref().map(|m| m.retention.clone()),
            jurisdiction: meta.as_ref().and_then(|m| m.jurisdiction.clone()),
            practice_area: meta.as_ref().and_then(|m| m.practice_area.clone()),
            opened_at: meta.as_ref().and_then(|m| m.opened_at.clone()),
        });
    }

    matters.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(Json(MattersListResponse { matters }))
}

fn parse_required_matter_field(
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

fn parse_optional_matter_field(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn validate_opened_at(value: &str) -> Result<(), (StatusCode, String)> {
    match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        Ok(parsed) if parsed.format("%Y-%m-%d").to_string() == value => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "'opened_at' must be in YYYY-MM-DD format".to_string(),
        )),
    }
}

fn parse_matter_list(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

fn list_matters_root_entries(
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

    let raw_matter_id = parse_required_matter_field("matter_id", &req.matter_id)?;
    let sanitized = crate::legal::policy::sanitize_matter_id(&raw_matter_id);
    if sanitized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Matter ID is empty after sanitization".to_string(),
        ));
    }

    let existing = list_matters_root_entries(workspace.list(MATTER_ROOT).await)?;
    let matter_prefix = format!("{MATTER_ROOT}/{sanitized}");
    if existing
        .iter()
        .any(|entry| entry.is_directory && entry.path == matter_prefix)
    {
        return Err((
            StatusCode::CONFLICT,
            format!("Matter '{}' already exists", sanitized),
        ));
    }

    let client = parse_required_matter_field("client", &req.client)?;
    let confidentiality = parse_required_matter_field("confidentiality", &req.confidentiality)?;
    let retention = parse_required_matter_field("retention", &req.retention)?;
    let jurisdiction = parse_optional_matter_field(req.jurisdiction);
    let practice_area = parse_optional_matter_field(req.practice_area);
    let opened_at = parse_optional_matter_field(req.opened_at);
    if let Some(value) = opened_at.as_deref() {
        validate_opened_at(value)?;
    }
    let team = parse_matter_list(req.team);
    let adversaries = parse_matter_list(req.adversaries);

    let metadata = crate::legal::matter::MatterMetadata {
        matter_id: sanitized.clone(),
        client: client.clone(),
        team: team.clone(),
        confidentiality: confidentiality.clone(),
        adversaries: adversaries.clone(),
        retention: retention.clone(),
        jurisdiction: jurisdiction.clone(),
        practice_area: practice_area.clone(),
        opened_at: opened_at.clone(),
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

    Ok((
        StatusCode::CREATED,
        Json(CreateMatterResponse {
            matter: MatterInfo {
                id: sanitized.clone(),
                client: Some(client),
                confidentiality: Some(confidentiality),
                team,
                adversaries,
                retention: Some(retention),
                jurisdiction,
                practice_area,
                opened_at,
            },
            active_matter_id: sanitized,
        }),
    ))
}

async fn matters_active_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ActiveMatterResponse>, (StatusCode, String)> {
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
            MATTER_ROOT,
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
                MATTER_ROOT,
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

async fn matter_documents_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<MatterDocumentsQuery>,
) -> Result<Json<MatterDocumentsResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let include_templates = query.include_templates.unwrap_or(false);
    let matter_prefix = format!("{MATTER_ROOT}/{matter_id}");
    let documents =
        list_matter_documents_recursive(workspace.as_ref(), &matter_prefix, include_templates)
            .await?;

    Ok(Json(MatterDocumentsResponse {
        matter_id,
        documents,
    }))
}

async fn matter_dashboard_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDashboardResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let matter_prefix = format!("{MATTER_ROOT}/{matter_id}");
    let docs = list_matter_documents_recursive(workspace.as_ref(), &matter_prefix, false).await?;
    let templates = list_matter_templates(workspace.as_ref(), &matter_id).await?;
    let today = Utc::now().date_naive();
    let deadlines = read_matter_deadlines(workspace.as_ref(), &matter_prefix, today).await?;

    let document_count = docs.iter().filter(|doc| !doc.is_dir).count();
    let draft_prefix = format!("{matter_prefix}/drafts/");
    let draft_count = docs
        .iter()
        .filter(|doc| !doc.is_dir && doc.path.starts_with(&draft_prefix))
        .count();

    let checklist_files = [
        format!("{matter_prefix}/workflows/intake_checklist.md"),
        format!("{matter_prefix}/workflows/review_and_filing_checklist.md"),
    ];
    let mut checklist_completed = 0usize;
    let mut checklist_total = 0usize;
    for path in checklist_files {
        match workspace.read(&path).await {
            Ok(doc) => {
                let (completed, total) = checklist_completion_from_markdown(&doc.content);
                checklist_completed += completed;
                checklist_total += total;
            }
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {}
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    let mut overdue_deadlines = 0usize;
    let mut upcoming_deadlines_14d = 0usize;
    let mut next_deadline: Option<(NaiveDate, MatterDeadlineInfo)> = None;
    for deadline in deadlines {
        let Ok(date) = NaiveDate::parse_from_str(&deadline.date, "%Y-%m-%d") else {
            continue;
        };
        if date < today {
            overdue_deadlines += 1;
            continue;
        }
        let days_until = date.signed_duration_since(today).num_days();
        if days_until <= 14 {
            upcoming_deadlines_14d += 1;
        }
        if next_deadline
            .as_ref()
            .is_none_or(|(existing, _)| date < *existing)
        {
            next_deadline = Some((date, deadline));
        }
    }

    Ok(Json(MatterDashboardResponse {
        matter_id,
        document_count,
        template_count: templates.len(),
        draft_count,
        checklist_completed,
        checklist_total,
        overdue_deadlines,
        upcoming_deadlines_14d,
        next_deadline: next_deadline.map(|(_, item)| item),
    }))
}

async fn matter_deadlines_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterDeadlinesResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let matter_prefix = format!("{MATTER_ROOT}/{matter_id}");
    let deadlines =
        read_matter_deadlines(workspace.as_ref(), &matter_prefix, Utc::now().date_naive()).await?;

    Ok(Json(MatterDeadlinesResponse {
        matter_id,
        deadlines,
    }))
}

async fn matter_templates_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<MatterTemplatesResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let templates = list_matter_templates(workspace.as_ref(), &matter_id).await?;

    Ok(Json(MatterTemplatesResponse {
        matter_id,
        templates,
    }))
}

async fn matter_template_apply_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(req): Json<MatterTemplateApplyRequest>,
) -> Result<(StatusCode, Json<MatterTemplateApplyResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let matter_prefix = format!("{MATTER_ROOT}/{matter_id}");
    let template_name = parse_template_name(&req.template_name)?;
    let template_path = format!("{matter_prefix}/templates/{template_name}");

    let template_doc = workspace
        .read(&template_path)
        .await
        .map_err(|err| match err {
            crate::error::WorkspaceError::DocumentNotFound { .. } => (
                StatusCode::NOT_FOUND,
                format!("Template '{}' not found", template_name),
            ),
            other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        })?;

    let timestamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let destination = choose_template_apply_destination(
        workspace.as_ref(),
        &matter_prefix,
        &template_name,
        &timestamp,
    )
    .await?;

    workspace
        .write(&destination, &template_doc.content)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(MatterTemplateApplyResponse {
            path: destination,
            status: "created",
        }),
    ))
}

async fn matter_filing_package_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<MatterFilingPackageResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let matter_id = ensure_existing_matter_for_route(workspace.as_ref(), &id).await?;
    let matter_prefix = format!("{MATTER_ROOT}/{matter_id}");
    let generated_at = Utc::now();
    let timestamp = generated_at.format("%Y%m%d-%H%M%S").to_string();
    let destination =
        choose_filing_package_destination(workspace.as_ref(), &matter_prefix, &timestamp).await?;

    let metadata = crate::legal::matter::read_matter_metadata_for_root(
        workspace.as_ref(),
        MATTER_ROOT,
        &matter_id,
    )
    .await
    .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let docs = list_matter_documents_recursive(workspace.as_ref(), &matter_prefix, true).await?;
    let templates = list_matter_templates(workspace.as_ref(), &matter_id).await?;
    let today = generated_at.date_naive();
    let deadlines = read_matter_deadlines(workspace.as_ref(), &matter_prefix, today).await?;

    let checklist_files = [
        format!("{matter_prefix}/workflows/intake_checklist.md"),
        format!("{matter_prefix}/workflows/review_and_filing_checklist.md"),
    ];
    let mut checklist_completed = 0usize;
    let mut checklist_total = 0usize;
    for path in checklist_files {
        match workspace.read(&path).await {
            Ok(doc) => {
                let (completed, total) = checklist_completion_from_markdown(&doc.content);
                checklist_completed += completed;
                checklist_total += total;
            }
            Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {}
            Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
        }
    }

    let mut package = String::new();
    package.push_str("# Filing Package Index\n\n");
    package.push_str(&format!("Matter: `{}`\n", matter_id));
    package.push_str(&format!("Client: {}\n", metadata.client));
    package.push_str(&format!("Confidentiality: {}\n", metadata.confidentiality));
    package.push_str(&format!("Generated: {}\n\n", generated_at.to_rfc3339()));

    let file_docs: Vec<&MatterDocumentInfo> = docs.iter().filter(|doc| !doc.is_dir).collect();
    let draft_prefix = format!("{matter_prefix}/drafts/");
    let draft_count = file_docs
        .iter()
        .filter(|doc| doc.path.starts_with(&draft_prefix))
        .count();
    let overdue_deadlines = deadlines.iter().filter(|item| item.is_overdue).count();
    let upcoming_deadlines_14d = deadlines
        .iter()
        .filter_map(|item| {
            NaiveDate::parse_from_str(&item.date, "%Y-%m-%d")
                .ok()
                .map(|date| date.signed_duration_since(today).num_days())
        })
        .filter(|days| (0..=14).contains(days))
        .count();

    package.push_str("## Workflow Scorecard\n\n");
    package.push_str(&format!("- Documents: {}\n", file_docs.len()));
    package.push_str(&format!("- Drafts: {}\n", draft_count));
    package.push_str(&format!("- Templates: {}\n", templates.len()));
    package.push_str(&format!(
        "- Checklist completion: {}/{}\n",
        checklist_completed, checklist_total
    ));
    package.push_str(&format!("- Overdue deadlines: {}\n", overdue_deadlines));
    package.push_str(&format!(
        "- Upcoming deadlines (14d): {}\n\n",
        upcoming_deadlines_14d
    ));

    package.push_str("## Deadlines Snapshot\n\n");
    if deadlines.is_empty() {
        package.push_str("- None parsed from `deadlines/calendar.md`.\n\n");
    } else {
        package.push_str("| Date | Event | Owner | Status | Source |\n");
        package.push_str("|---|---|---|---|---|\n");
        for item in &deadlines {
            package.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                item.date,
                item.title.replace('|', "\\|"),
                item.owner.clone().unwrap_or_default().replace('|', "\\|"),
                item.status.clone().unwrap_or_default().replace('|', "\\|"),
                item.source.clone().unwrap_or_default().replace('|', "\\|"),
            ));
        }
        package.push('\n');
    }

    package.push_str("## Document Inventory\n\n");
    if file_docs.is_empty() {
        package.push_str("- No documents found.\n\n");
    } else {
        for doc in &file_docs {
            package.push_str(&format!("- `{}`\n", doc.path));
        }
        package.push('\n');
    }

    package.push_str("## Template Inventory\n\n");
    if templates.is_empty() {
        package.push_str("- No templates found.\n");
    } else {
        for template in &templates {
            package.push_str(&format!("- `{}`\n", template.path));
        }
    }

    workspace
        .write(&destination, &package)
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(MatterFilingPackageResponse {
            matter_id,
            path: destination,
            generated_at: generated_at.to_rfc3339(),
            status: "created",
        }),
    ))
}

async fn matters_conflicts_check_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MatterConflictCheckRequest>,
) -> Result<Json<MatterConflictCheckResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let text = req.text.trim();
    if text.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "'text' must not be empty".to_string(),
        ));
    }

    let mut legal = legal_config_for_gateway(state.as_ref());
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
        load_active_matter_for_chat(state.as_ref()).await
    };

    legal.active_matter = effective_matter_id.clone();

    match workspace.read("conflicts.json").await {
        Ok(_) | Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {}
        Err(err) => return Err((StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }

    let conflict = crate::legal::matter::detect_conflict(workspace.as_ref(), &legal, text).await;
    Ok(Json(MatterConflictCheckResponse {
        matched: conflict.is_some(),
        conflict,
        matter_id: effective_matter_id,
    }))
}

async fn legal_audit_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<LegalAuditQuery>,
) -> Result<Json<LegalAuditListResponse>, (StatusCode, String)> {
    let legal = legal_config_for_gateway(state.as_ref());
    if !legal.audit.enabled {
        return Err((
            StatusCode::NOT_FOUND,
            "Legal audit logging is disabled".to_string(),
        ));
    }

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
    let from_ts = parse_utc_query_ts("from", query.from.as_deref())?;
    let to_ts = parse_utc_query_ts("to", query.to.as_deref())?;

    if let (Some(from), Some(to)) = (from_ts, to_ts)
        && from > to
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "'from' must be earlier than or equal to 'to'".to_string(),
        ));
    }

    let path = &legal.audit.path;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(LegalAuditListResponse {
                events: Vec::new(),
                total: 0,
                next_offset: None,
                parse_errors: 0,
                truncated: false,
            }));
        }
        Err(err) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to open legal audit log {:?}: {}", path, err),
            ));
        }
    };

    let mut parse_errors = 0usize;
    let mut truncated = false;
    let mut filtered: Vec<LegalAuditEventInfo> = Vec::new();

    for (idx, line_res) in BufReader::new(file).lines().enumerate() {
        if idx >= MAX_AUDIT_SCAN_LINES {
            truncated = true;
            break;
        }
        let line_no = idx + 1;
        let line = line_res.map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to read legal audit log {:?}: {}", path, err),
            )
        })?;
        if line.trim().is_empty() {
            continue;
        }

        let parsed: LegalAuditEventLine = match serde_json::from_str(&line) {
            Ok(event) => event,
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };
        let ts = match DateTime::parse_from_rfc3339(&parsed.ts) {
            Ok(ts) => ts.with_timezone(&Utc),
            Err(_) => {
                parse_errors += 1;
                continue;
            }
        };

        if let Some(ref wanted) = event_type_filter
            && &parsed.event_type != wanted
        {
            continue;
        }
        if let Some(from) = from_ts
            && ts < from
        {
            continue;
        }
        if let Some(to) = to_ts
            && ts > to
        {
            continue;
        }

        filtered.push(LegalAuditEventInfo {
            line_no,
            ts: parsed.ts,
            event_type: parsed.event_type,
            details: parsed.details,
            metrics: parsed.metrics,
            prev_hash: parsed.prev_hash,
            hash: parsed.hash,
        });
    }

    let total = filtered.len();
    let events: Vec<LegalAuditEventInfo> = filtered.into_iter().skip(offset).take(limit).collect();
    let next_offset = if offset + events.len() < total {
        Some(offset + events.len())
    } else {
        None
    };

    Ok(Json(LegalAuditListResponse {
        events,
        total,
        next_offset,
        parse_errors,
        truncated,
    }))
}

// --- Jobs handlers ---

async fn jobs_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<JobListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Fetch sandbox jobs scoped to the authenticated user.
    let sandbox_jobs = store
        .list_sandbox_jobs_for_user(&state.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Scope jobs to the authenticated user.
    let mut jobs: Vec<JobInfo> = sandbox_jobs
        .iter()
        .filter(|j| j.user_id == state.user_id)
        .map(|j| {
            let ui_state = match j.status.as_str() {
                "creating" => "pending",
                "running" => "in_progress",
                s => s,
            };
            JobInfo {
                id: j.id,
                title: j.task.clone(),
                state: ui_state.to_string(),
                user_id: j.user_id.clone(),
                created_at: j.created_at.to_rfc3339(),
                started_at: j.started_at.map(|dt| dt.to_rfc3339()),
            }
        })
        .collect();

    // Most recent first.
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(JobListResponse { jobs }))
}

async fn jobs_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<JobSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let s = store
        .sandbox_job_summary_for_user(&state.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(JobSummaryResponse {
        total: s.total,
        pending: s.creating,
        in_progress: s.running,
        completed: s.completed,
        failed: s.failed + s.interrupted,
        stuck: 0,
    }))
}

async fn jobs_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<JobDetailResponse>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job from DB first, scoped to the authenticated user.
    if let Some(ref store) = state.store
        && let Ok(Some(job)) = store.get_sandbox_job(job_id).await
    {
        if job.user_id != state.user_id {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }
        let browse_id = std::path::Path::new(&job.project_dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| job.id.to_string());

        let ui_state = match job.status.as_str() {
            "creating" => "pending",
            "running" => "in_progress",
            s => s,
        };

        let elapsed_secs = job.started_at.map(|start| {
            let end = job.completed_at.unwrap_or_else(chrono::Utc::now);
            (end - start).num_seconds().max(0) as u64
        });

        // Synthesize transitions from timestamps.
        let mut transitions = Vec::new();
        if let Some(started) = job.started_at {
            transitions.push(TransitionInfo {
                from: "creating".to_string(),
                to: "running".to_string(),
                timestamp: started.to_rfc3339(),
                reason: None,
            });
        }
        if let Some(completed) = job.completed_at {
            transitions.push(TransitionInfo {
                from: "running".to_string(),
                to: job.status.clone(),
                timestamp: completed.to_rfc3339(),
                reason: job.failure_reason.clone(),
            });
        }

        return Ok(Json(JobDetailResponse {
            id: job.id,
            title: job.task.clone(),
            description: String::new(),
            state: ui_state.to_string(),
            user_id: job.user_id.clone(),
            created_at: job.created_at.to_rfc3339(),
            started_at: job.started_at.map(|dt| dt.to_rfc3339()),
            completed_at: job.completed_at.map(|dt| dt.to_rfc3339()),
            elapsed_secs,
            project_dir: Some(job.project_dir.clone()),
            browse_url: Some(format!("/projects/{}/", browse_id)),
            job_mode: {
                let mode = store.get_sandbox_job_mode(job.id).await.ok().flatten();
                mode.filter(|m| m != "worker")
            },
            transitions,
        }));
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
}

async fn jobs_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job cancellation, scoped to the authenticated user.
    if let Some(ref store) = state.store
        && let Ok(Some(job)) = store.get_sandbox_job(job_id).await
    {
        if job.user_id != state.user_id {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }
        if job.status == "running" || job.status == "creating" {
            // Stop the container if we have a job manager.
            if let Some(ref jm) = state.job_manager
                && let Err(e) = jm.stop_job(job_id).await
            {
                tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container during cancellation");
            }
            store
                .update_sandbox_job_status(
                    job_id,
                    "failed",
                    Some(false),
                    Some("Cancelled by user"),
                    None,
                    Some(chrono::Utc::now()),
                )
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        return Ok(Json(serde_json::json!({
            "status": "cancelled",
            "job_id": job_id,
        })));
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
}

async fn jobs_restart_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let jm = state.job_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Sandbox not enabled".to_string(),
    ))?;

    let old_job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let old_job = store
        .get_sandbox_job(old_job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Scope to the authenticated user.
    if old_job.user_id != state.user_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    if old_job.status != "interrupted" && old_job.status != "failed" {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot restart job in state '{}'", old_job.status),
        ));
    }

    // Create a new job with the same task and project_dir.
    let new_job_id = Uuid::new_v4();
    let now = chrono::Utc::now();

    let record = crate::history::SandboxJobRecord {
        id: new_job_id,
        task: old_job.task.clone(),
        status: "creating".to_string(),
        user_id: old_job.user_id.clone(),
        project_dir: old_job.project_dir.clone(),
        success: None,
        failure_reason: None,
        created_at: now,
        started_at: None,
        completed_at: None,
        credential_grants_json: old_job.credential_grants_json.clone(),
    };
    store
        .save_sandbox_job(&record)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Look up the original job's mode so the restart uses the same mode.
    let mode = match store.get_sandbox_job_mode(old_job_id).await {
        Ok(Some(m)) if m == "claude_code" => crate::orchestrator::job_manager::JobMode::ClaudeCode,
        _ => crate::orchestrator::job_manager::JobMode::Worker,
    };

    // Restore credential grants from the original job so the restarted container
    // has access to the same secrets.
    let credential_grants: Vec<crate::orchestrator::auth::CredentialGrant> =
        serde_json::from_str(&old_job.credential_grants_json).unwrap_or_else(|e| {
            tracing::warn!(
                job_id = %old_job.id,
                "Failed to deserialize credential grants from stored job: {}. \
                 Restarted job will have no credentials.",
                e
            );
            vec![]
        });

    let project_dir = std::path::PathBuf::from(&old_job.project_dir);
    let _token = jm
        .create_job(
            new_job_id,
            &old_job.task,
            Some(project_dir),
            mode,
            credential_grants,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create container: {}", e),
            )
        })?;

    store
        .update_sandbox_job_status(new_job_id, "running", None, None, Some(now), None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "restarted",
        "old_job_id": old_job_id,
        "new_job_id": new_job_id,
    })))
}

// --- Claude Code prompt and events handlers ---

/// Submit a follow-up prompt to a running Claude Code sandbox job.
async fn jobs_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let prompt_queue = state.prompt_queue.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Claude Code not configured".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Verify user owns this job.
    if let Some(ref store) = state.store
        && !store
            .sandbox_job_belongs_to_user(job_id, &state.user_id)
            .await
            .unwrap_or(false)
    {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing 'content' field".to_string(),
        ))?
        .to_string();

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    let prompt = crate::orchestrator::api::PendingPrompt { content, done };

    {
        let mut queue = prompt_queue.lock().await;
        queue.entry(job_id).or_default().push_back(prompt);
    }

    Ok(Json(serde_json::json!({
        "status": "queued",
        "job_id": job_id.to_string(),
    })))
}

/// Load persisted job events for a job (for history replay on page open).
async fn jobs_events_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Database not available".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Verify user owns this job.
    if !store
        .sandbox_job_belongs_to_user(job_id, &state.user_id)
        .await
        .unwrap_or(false)
    {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let events = store
        .list_job_events(job_id, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let events_json: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "event_type": e.event_type,
                "data": e.data,
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "job_id": job_id.to_string(),
        "events": events_json,
    })))
}

// --- Project file handlers for sandbox jobs ---

#[derive(Deserialize)]
struct FilePathQuery {
    path: Option<String>,
}

async fn job_files_list_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFilesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Verify user owns this job.
    if job.user_id != state.user_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let base = std::path::PathBuf::from(&job.project_dir);
    let rel_path = query.path.as_deref().unwrap_or("");
    let target = base.join(rel_path);

    // Path traversal guard.
    let canonical = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read directory".to_string()))?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let rel = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel_path, name)
        };
        entries.push(ProjectFileEntry {
            name,
            path: rel,
            is_dir,
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    Ok(Json(ProjectFilesResponse { entries }))
}

async fn job_files_read_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFileReadResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    // Verify user owns this job.
    if job.user_id != state.user_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let path = query.path.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "path parameter required".to_string(),
    ))?;

    let base = std::path::PathBuf::from(&job.project_dir);
    let file_path = base.join(path);

    let canonical = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read file".to_string()))?;

    Ok(Json(ProjectFileReadResponse {
        path: path.to_string(),
        content,
    }))
}

// --- Logs handlers ---

async fn logs_events_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let broadcaster = state.log_broadcaster.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log broadcaster not available".to_string(),
    ))?;

    // Replay recent history so late-joining browsers see startup logs.
    // Subscribe BEFORE snapshotting to avoid a gap between history and live.
    let rx = broadcaster.subscribe();
    let history = broadcaster.recent_entries();

    let history_stream = futures::stream::iter(history).map(|entry| {
        let data = serde_json::to_string(&entry).unwrap_or_default();
        Ok::<_, Infallible>(Event::default().event("log").data(data))
    });

    let live_stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|result| result.ok())
        .map(|entry| {
            let data = serde_json::to_string(&entry).unwrap_or_default();
            Ok::<_, Infallible>(Event::default().event("log").data(data))
        });

    let stream = history_stream.chain(live_stream);

    Ok((
        [("X-Accel-Buffering", "no"), ("Cache-Control", "no-cache")],
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(30))
                .text(""),
        ),
    ))
}

async fn logs_level_get_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

async fn logs_level_set_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let handle = state.log_level_handle.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Log level control not available".to_string(),
    ))?;

    let level = body
        .get("level")
        .and_then(|v| v.as_str())
        .ok_or((StatusCode::BAD_REQUEST, "missing 'level' field".to_string()))?;

    handle
        .set_level(level)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    tracing::info!("Log level changed to '{}'", handle.current_level());
    Ok(Json(serde_json::json!({ "level": handle.current_level() })))
}

// --- Extension handlers ---

async fn extensions_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ExtensionListResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let installed = ext_mgr
        .list(None, false)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let extensions = installed
        .into_iter()
        .map(|ext| ExtensionInfo {
            name: ext.name,
            kind: ext.kind.to_string(),
            description: ext.description,
            url: ext.url,
            authenticated: ext.authenticated,
            active: ext.active,
            tools: ext.tools,
            needs_setup: ext.needs_setup,
        })
        .collect();

    Ok(Json(ExtensionListResponse { extensions }))
}

async fn extensions_tools_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<ToolListResponse>, (StatusCode, String)> {
    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Tool registry not available".to_string(),
    ))?;

    let definitions = registry.tool_definitions().await;
    let tools = definitions
        .into_iter()
        .map(|td| ToolInfo {
            name: td.name,
            description: td.description,
        })
        .collect();

    Ok(Json(ToolListResponse { tools }))
}

async fn extensions_install_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<InstallExtensionRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    // When extension manager isn't available, check registry entries for a helpful message
    let Some(ext_mgr) = state.extension_manager.as_ref() else {
        // Look up the entry in the catalog to give a specific error
        if let Some(entry) = state.registry_entries.iter().find(|e| e.name == req.name) {
            let msg = match &entry.source {
                crate::extensions::ExtensionSource::WasmBuildable { .. } => {
                    format!(
                        "'{}' requires building from source. \
                         Run `clawyer registry install {}` from the CLI.",
                        req.name, req.name
                    )
                }
                _ => format!(
                    "Extension manager not available (secrets store required). \
                     Configure DATABASE_URL or a secrets backend to enable installation of '{}'.",
                    req.name
                ),
            };
            return Ok(Json(ActionResponse::fail(msg)));
        }
        return Ok(Json(ActionResponse::fail(
            "Extension manager not available (secrets store required)".to_string(),
        )));
    };

    let kind_hint = req.kind.as_deref().and_then(|k| match k {
        "mcp_server" => Some(crate::extensions::ExtensionKind::McpServer),
        "wasm_tool" => Some(crate::extensions::ExtensionKind::WasmTool),
        "wasm_channel" => Some(crate::extensions::ExtensionKind::WasmChannel),
        _ => None,
    });

    match ext_mgr
        .install(&req.name, req.url.as_deref(), kind_hint)
        .await
    {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_activate_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.activate(&name).await {
        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
        Err(activate_err) => {
            let err_str = activate_err.to_string();
            let needs_auth = err_str.contains("authentication")
                || err_str.contains("401")
                || err_str.contains("Unauthorized");

            if !needs_auth {
                return Ok(Json(ActionResponse::fail(err_str)));
            }

            // Activation failed due to auth; try authenticating first.
            match ext_mgr.auth(&name, None).await {
                Ok(auth_result) if auth_result.status == "authenticated" => {
                    // Auth succeeded, retry activation.
                    match ext_mgr.activate(&name).await {
                        Ok(result) => Ok(Json(ActionResponse::ok(result.message))),
                        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
                    }
                }
                Ok(auth_result) => {
                    // Auth in progress (OAuth URL or awaiting manual token).
                    let mut resp = ActionResponse::fail(
                        auth_result
                            .instructions
                            .clone()
                            .unwrap_or_else(|| format!("'{}' requires authentication.", name)),
                    );
                    resp.auth_url = auth_result.auth_url;
                    resp.awaiting_token = Some(auth_result.awaiting_token);
                    resp.instructions = auth_result.instructions;
                    Ok(Json(resp))
                }
                Err(auth_err) => Ok(Json(ActionResponse::fail(format!(
                    "Authentication failed: {}",
                    auth_err
                )))),
            }
        }
    }
}

// --- Project file serving handlers ---

/// Redirect `/projects/{id}` to `/projects/{id}/` so relative paths in
/// the served HTML resolve within the project namespace.
async fn project_redirect_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    axum::response::Redirect::permanent(&format!("/projects/{project_id}/"))
}

/// Serve `index.html` when hitting `/projects/{project_id}/`.
async fn project_index_handler(Path(project_id): Path<String>) -> impl IntoResponse {
    serve_project_file(&project_id, "index.html").await
}

/// Serve any file under `/projects/{project_id}/{path}`.
async fn project_file_handler(
    Path((project_id, path)): Path<(String, String)>,
) -> impl IntoResponse {
    serve_project_file(&project_id, &path).await
}

/// Shared logic: resolve the file inside `~/.clawyer/projects/{project_id}/`,
/// guard against path traversal, and stream the content with the right MIME type.
async fn serve_project_file(project_id: &str, path: &str) -> axum::response::Response {
    // Reject project_id values that could escape the projects directory.
    if project_id.contains('/')
        || project_id.contains('\\')
        || project_id.contains("..")
        || project_id.is_empty()
    {
        return (StatusCode::BAD_REQUEST, "Invalid project ID").into_response();
    }

    let base = dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".clawyer")
        .join("projects")
        .join(project_id);

    let file_path = base.join(path);

    // Path traversal guard
    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    let base_canonical = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };
    if !canonical.starts_with(&base_canonical) {
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    match tokio::fs::read(&canonical).await {
        Ok(contents) => {
            let mime = mime_guess::from_path(&canonical)
                .first_or_octet_stream()
                .to_string();
            ([(header::CONTENT_TYPE, mime)], contents).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

async fn extensions_remove_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.remove(&name).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

async fn extensions_registry_handler(
    State(state): State<Arc<GatewayState>>,
    Query(params): Query<RegistrySearchQuery>,
) -> Json<RegistrySearchResponse> {
    let query = params.query.unwrap_or_default();
    let query_lower = query.to_lowercase();
    let tokens: Vec<&str> = query_lower.split_whitespace().collect();

    // Filter registry entries by query (or return all if empty)
    let matching: Vec<&crate::extensions::RegistryEntry> = if tokens.is_empty() {
        state.registry_entries.iter().collect()
    } else {
        state
            .registry_entries
            .iter()
            .filter(|e| {
                let name = e.name.to_lowercase();
                let display = e.display_name.to_lowercase();
                let desc = e.description.to_lowercase();
                tokens.iter().any(|t| {
                    name.contains(t)
                        || display.contains(t)
                        || desc.contains(t)
                        || e.keywords.iter().any(|k| k.to_lowercase().contains(t))
                })
            })
            .collect()
    };

    // Cross-reference with installed extensions by (name, kind) to avoid
    // false positives when the same name exists as different kinds.
    let installed: std::collections::HashSet<(String, String)> =
        if let Some(ext_mgr) = state.extension_manager.as_ref() {
            ext_mgr
                .list(None, false)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|ext| (ext.name, ext.kind.to_string()))
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    let entries = matching
        .into_iter()
        .map(|e| {
            let kind_str = e.kind.to_string();
            RegistryEntryInfo {
                name: e.name.clone(),
                display_name: e.display_name.clone(),
                installed: installed.contains(&(e.name.clone(), kind_str.clone())),
                kind: kind_str,
                description: e.description.clone(),
                keywords: e.keywords.clone(),
            }
        })
        .collect();

    Json(RegistrySearchResponse { entries })
}

async fn extensions_setup_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
) -> Result<Json<ExtensionSetupResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    let secrets = ext_mgr
        .get_setup_schema(&name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let kind = ext_mgr
        .list(None, false)
        .await
        .ok()
        .and_then(|list| list.into_iter().find(|e| e.name == name))
        .map(|e| e.kind.to_string())
        .unwrap_or_default();

    Ok(Json(ExtensionSetupResponse {
        name,
        kind,
        secrets,
    }))
}

async fn extensions_setup_submit_handler(
    State(state): State<Arc<GatewayState>>,
    Path(name): Path<String>,
    Json(req): Json<ExtensionSetupRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let ext_mgr = state.extension_manager.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Extension manager not available (secrets store required)".to_string(),
    ))?;

    match ext_mgr.save_setup_secrets(&name, &req.secrets).await {
        Ok(message) => Ok(Json(ActionResponse::ok(message))),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

// --- Pairing handlers ---

async fn pairing_list_handler(
    Path(channel): Path<String>,
) -> Result<Json<PairingListResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    let requests = store
        .list_pending(&channel)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let infos = requests
        .into_iter()
        .map(|r| PairingRequestInfo {
            code: r.code,
            sender_id: r.id,
            meta: r.meta,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(PairingListResponse {
        channel,
        requests: infos,
    }))
}

async fn pairing_approve_handler(
    Path(channel): Path<String>,
    Json(req): Json<PairingApproveRequest>,
) -> Result<Json<ActionResponse>, (StatusCode, String)> {
    let store = crate::pairing::PairingStore::new();
    match store.approve(&channel, &req.code) {
        Ok(Some(approved)) => Ok(Json(ActionResponse::ok(format!(
            "Pairing approved for sender '{}'",
            approved.id
        )))),
        Ok(None) => Ok(Json(ActionResponse::fail(
            "Invalid or expired pairing code".to_string(),
        ))),
        Err(crate::pairing::PairingStoreError::ApproveRateLimited) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed approve attempts; try again later".to_string(),
        )),
        Err(e) => Ok(Json(ActionResponse::fail(e.to_string()))),
    }
}

// --- Routines handlers ---

async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines(&state.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(routine_to_info).collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines(&state.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let enabled = routines.iter().filter(|r| r.enabled).count() as u64;
    let disabled = total - enabled;
    let failing = routines
        .iter()
        .filter(|r| r.consecutive_failures > 0)
        .count() as u64;

    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc());
    let runs_today = if let Some(start) = today_start {
        routines
            .iter()
            .filter(|r| r.last_run_at.is_some_and(|ts| ts >= start))
            .count() as u64
    } else {
        0
    };

    Ok(Json(RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }))
}

async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<RoutineDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_runs: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(RoutineDetailResponse {
        id: routine.id,
        name: routine.name.clone(),
        description: routine.description.clone(),
        enabled: routine.enabled,
        trigger: serde_json::to_value(&routine.trigger).unwrap_or_default(),
        action: serde_json::to_value(&routine.action).unwrap_or_default(),
        guardrails: serde_json::to_value(&routine.guardrails).unwrap_or_default(),
        notify: serde_json::to_value(&routine.notify).unwrap_or_default(),
        last_run_at: routine.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: routine.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: routine.run_count,
        consecutive_failures: routine.consecutive_failures,
        created_at: routine.created_at.to_rfc3339(),
        recent_runs,
    }))
}

async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    // Send the routine prompt through the message pipeline as a manual trigger.
    let prompt = match &routine.action {
        crate::agent::routine::RoutineAction::Lightweight { prompt, .. } => prompt.clone(),
        crate::agent::routine::RoutineAction::FullJob {
            title, description, ..
        } => format!("{}: {}", title, description),
    };

    let content = format!("[routine:{}] {}", routine.name, prompt);
    let msg = IncomingMessage::new("gateway", &state.user_id, content);

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

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine_id,
    })))
}

#[derive(Deserialize)]
struct ToggleRequest {
    enabled: Option<bool>,
}

async fn routines_toggle_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<ToggleRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let mut routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    // If a specific value was provided, use it; otherwise toggle.
    routine.enabled = match body {
        Some(Json(req)) => req.enabled.unwrap_or(!routine.enabled),
        None => !routine.enabled,
    };

    store
        .update_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(Json(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Routine not found".to_string()))
    }
}

async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

/// Convert a Routine to the trimmed RoutineInfo for list display.
fn routine_to_info(r: &crate::agent::routine::Routine) -> RoutineInfo {
    let (trigger_type, trigger_summary) = match &r.trigger {
        crate::agent::routine::Trigger::Cron { schedule } => {
            ("cron".to_string(), format!("cron: {}", schedule))
        }
        crate::agent::routine::Trigger::Event {
            pattern, channel, ..
        } => {
            let ch = channel.as_deref().unwrap_or("any");
            ("event".to_string(), format!("on {} /{}/", ch, pattern))
        }
        crate::agent::routine::Trigger::Webhook { path, .. } => {
            let p = path.as_deref().unwrap_or("/");
            ("webhook".to_string(), format!("webhook: {}", p))
        }
        crate::agent::routine::Trigger::Manual => ("manual".to_string(), "manual only".to_string()),
    };

    let action_type = match &r.action {
        crate::agent::routine::RoutineAction::Lightweight { .. } => "lightweight",
        crate::agent::routine::RoutineAction::FullJob { .. } => "full_job",
    };

    let status = if !r.enabled {
        "disabled"
    } else if r.consecutive_failures > 0 {
        "failing"
    } else {
        "active"
    };

    RoutineInfo {
        id: r.id,
        name: r.name.clone(),
        description: r.description.clone(),
        enabled: r.enabled,
        trigger_type,
        trigger_summary,
        action_type: action_type.to_string(),
        last_run_at: r.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: r.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: r.run_count,
        consecutive_failures: r.consecutive_failures,
        status: status.to_string(),
    }
}

async fn routines_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<RoutineCreateRequest>,
) -> Result<(StatusCode, Json<RoutineInfo>), (StatusCode, String)> {
    use crate::agent::routine::{
        NotifyConfig, Routine, RoutineAction, RoutineGuardrails, Trigger, next_cron_fire,
    };

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let name = req.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "name is required".to_string()));
    }

    // Build trigger
    let trigger = match req.trigger_type.as_str() {
        "cron" => {
            let schedule = req.schedule.as_deref().ok_or((
                StatusCode::BAD_REQUEST,
                "schedule is required for cron trigger".to_string(),
            ))?;
            next_cron_fire(schedule).map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("invalid cron schedule: {e}"),
                )
            })?;
            Trigger::Cron {
                schedule: schedule.to_string(),
            }
        }
        "event" => {
            let pattern = req.event_pattern.as_deref().ok_or((
                StatusCode::BAD_REQUEST,
                "event_pattern is required for event trigger".to_string(),
            ))?;
            regex::Regex::new(pattern)
                .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid regex: {e}")))?;
            Trigger::Event {
                pattern: pattern.to_string(),
                channel: req.event_channel.clone(),
            }
        }
        "webhook" => Trigger::Webhook {
            path: None,
            secret: None,
        },
        "manual" => Trigger::Manual,
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown trigger_type: {other}"),
            ));
        }
    };

    // Compute next fire time for cron triggers
    let next_fire = if let Trigger::Cron { ref schedule } = trigger {
        next_cron_fire(schedule).unwrap_or(None)
    } else {
        None
    };

    // Build action
    let action_type = req.action_type.as_deref().unwrap_or("lightweight");
    let context_paths = req.context_paths.unwrap_or_default();
    let action = match action_type {
        "lightweight" => RoutineAction::Lightweight {
            prompt: req.prompt.clone(),
            context_paths,
            max_tokens: 4096,
        },
        "full_job" => RoutineAction::FullJob {
            title: name.clone(),
            description: req.prompt.clone(),
            max_iterations: 10,
        },
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("unknown action_type: {other}"),
            ));
        }
    };

    let cooldown_secs = req.cooldown_secs.unwrap_or(300);
    let now = chrono::Utc::now();
    let routine = Routine {
        id: Uuid::new_v4(),
        name: name.clone(),
        description: req.description.unwrap_or_default(),
        user_id: state.user_id.clone(),
        enabled: true,
        trigger,
        action,
        guardrails: RoutineGuardrails {
            cooldown: std::time::Duration::from_secs(cooldown_secs),
            max_concurrent: 1,
            dedup_window: None,
        },
        notify: NotifyConfig::default(),
        last_run_at: None,
        next_fire_at: next_fire,
        run_count: 0,
        consecutive_failures: 0,
        state: serde_json::json!({}),
        created_at: now,
        updated_at: now,
    };

    store
        .create_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(routine_to_info(&routine))))
}

// --- Settings handlers ---

async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store.list_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to list settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let settings = rows
        .into_iter()
        .map(|r| SettingResponse {
            key: r.key,
            value: r.value,
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(SettingsListResponse { settings }))
}

async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SettingResponse {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

async fn settings_set_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .set_setting(&state.user_id, &key, &body.value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

async fn settings_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .delete_setting(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = store.get_all_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to export settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(SettingsExportResponse { settings }))
}

async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .set_all_settings(&state.user_id, &body.settings)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// --- Gateway control plane handlers ---

async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();

    let (daily_cost, actions_this_hour, model_usage) = if let Some(ref cg) = state.cost_guard {
        let cost = cg.daily_spend().await;
        let actions = cg.actions_this_hour().await;
        let usage = cg.model_usage().await;
        let models: Vec<ModelUsageEntry> = usage
            .into_iter()
            .map(|(model, tokens)| ModelUsageEntry {
                model,
                input_tokens: tokens.input_tokens,
                output_tokens: tokens.output_tokens,
                cost: format!("{:.6}", tokens.cost),
            })
            .collect();
        (Some(format!("{:.4}", cost)), Some(actions), Some(models))
    } else {
        (None, None, None)
    };

    Json(GatewayStatusResponse {
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        daily_cost,
        actions_this_hour,
        model_usage,
    })
}

#[derive(serde::Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(serde::Serialize)]
struct GatewayStatusResponse {
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;
    use regex::Regex;

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
    async fn matters_create_creates_scaffold_and_sets_active() {
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
                opened_at: Some("2024-03-15".to_string()),
                team: vec!["Lead Counsel".to_string()],
                adversaries: vec!["Foo LLC".to_string()],
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
        assert_eq!(response.matter.opened_at.as_deref(), Some("2024-03-15"));

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
        assert_eq!(parsed.opened_at.as_deref(), Some("2024-03-15"));
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
                opened_at: Some("2024-03-15".to_string()),
                team: vec!["Lead Counsel".to_string()],
                adversaries: vec!["Foo LLC".to_string()],
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
        assert_eq!(matter.opened_at.as_deref(), Some("2024-03-15"));
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
                opened_at: None,
                team: vec![],
                adversaries: vec![],
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
                opened_at: None,
                team: vec![],
                adversaries: vec![],
            }),
        )
        .await
        .expect_err("duplicate should fail");

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("already exists"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn matters_create_rejects_invalid_opened_at() {
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
                opened_at: Some("03/15/2024".to_string()),
                team: vec![],
                adversaries: vec![],
            }),
        )
        .await
        .expect_err("invalid opened_at should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("YYYY-MM-DD"));
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
                opened_at: None,
                team: vec![],
                adversaries: vec![],
            }),
        )
        .await
        .expect_err("invalid matter id should fail");

        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        assert!(err.1.contains("empty after sanitization"));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn conflicts_check_returns_hit_for_matching_entry() {
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
    async fn legal_audit_list_returns_empty_when_missing() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let dir = tempfile::tempdir().expect("tempdir");

        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        legal.audit.path = dir.path().join("missing-audit.jsonl");
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = legal_audit_list_handler(State(state), Query(LegalAuditQuery::default()))
            .await
            .expect("missing file should not error");

        assert!(resp.events.is_empty());
        assert_eq!(resp.total, 0);
        assert_eq!(resp.next_offset, None);
        assert_eq!(resp.parse_errors, 0);
        assert!(!resp.truncated);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_audit_list_supports_filters_and_paging() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");

        let mut file = fs::File::create(&path).expect("create audit file");
        writeln!(
            file,
            r#"{{"ts":"2026-01-01T00:00:00Z","event_type":"prompt_received","details":{{}},"metrics":{{}}}}"#
        )
        .expect("write line");
        writeln!(
            file,
            r#"{{"ts":"2026-01-02T00:00:00Z","event_type":"approval_required","details":{{"id":1}},"metrics":{{"approval_required":1}}}}"#
        )
        .expect("write line");
        writeln!(
            file,
            r#"{{"ts":"2026-01-03T00:00:00Z","event_type":"approval_required","details":{{"id":2}},"metrics":{{"approval_required":2}}}}"#
        )
        .expect("write line");
        writeln!(
            file,
            r#"{{"ts":"2026-01-04T00:00:00Z","event_type":"approval_required","details":{{"id":3}},"metrics":{{"approval_required":3}}}}"#
        )
        .expect("write line");

        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        legal.audit.path = path;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = legal_audit_list_handler(
            State(state),
            Query(LegalAuditQuery {
                limit: Some(1),
                offset: Some(0),
                event_type: Some("approval_required".to_string()),
                from: Some("2026-01-02T00:00:00Z".to_string()),
                to: Some("2026-01-03T23:59:59Z".to_string()),
            }),
        )
        .await
        .expect("audit list should succeed");

        assert_eq!(resp.total, 2);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.next_offset, Some(1));
        assert_eq!(resp.events[0].line_no, 2);
        assert_eq!(resp.events[0].event_type, "approval_required");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn legal_audit_list_tracks_parse_errors() {
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");

        let mut file = fs::File::create(&path).expect("create audit file");
        writeln!(
            file,
            r#"{{"ts":"2026-01-01T00:00:00Z","event_type":"prompt_received","details":{{}},"metrics":{{}}}}"#
        )
        .expect("write valid line");
        writeln!(file, "not-json").expect("write invalid json line");
        writeln!(
            file,
            r#"{{"ts":"not-a-timestamp","event_type":"prompt_received","details":{{}},"metrics":{{}}}}"#
        )
        .expect("write invalid ts line");

        let mut legal = test_legal_config();
        legal.audit.enabled = true;
        legal.audit.path = path;
        let state = test_gateway_state_with_store_workspace_and_legal(db, workspace, legal);

        let Json(resp) = legal_audit_list_handler(State(state), Query(LegalAuditQuery::default()))
            .await
            .expect("audit list should succeed");

        assert_eq!(resp.total, 1);
        assert_eq!(resp.events.len(), 1);
        assert_eq!(resp.parse_errors, 2);
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
    async fn memory_write_handler_invalidates_conflict_cache() {
        crate::legal::matter::reset_conflict_cache_for_tests();
        let (db, _tmp) = crate::testing::test_db().await;
        let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
        let state =
            test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

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
