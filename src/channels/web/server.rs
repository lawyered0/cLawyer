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

use crate::channels::web::auth::{
    AuthPrincipal, AuthState, auth_middleware, compute_token_hash, derive_token_hmac_key,
};
pub use crate::channels::web::state::{GatewayState, PromptQueue, RateLimiter};
use crate::channels::web::types::*;
use crate::db::UserRole;

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
    let principal = resolve_gateway_principal(state.as_ref()).await?;
    // Derive a stable HMAC-SHA256 key from the gateway auth token. The key is
    // deterministic (same token → same key) so no extra configuration is needed.
    // Rotating the auth token automatically invalidates stored hashes; the
    // upsert below immediately writes the new HMAC hash on startup.
    let hmac_key = derive_token_hmac_key(&auth_token);
    if let Some(store) = state.store.as_ref() {
        let token_hash = compute_token_hash(&auth_token, Some(&hmac_key));
        store
            .upsert_user_token_hash(&principal.user_id, &token_hash)
            .await
            .map_err(|err| crate::error::ChannelError::StartupFailed {
                name: "gateway".to_string(),
                reason: format!(
                    "Failed to persist gateway auth token mapping for user '{}': {}",
                    principal.user_id, err
                ),
            })?;
    }
    let auth_state = AuthState {
        token: auth_token,
        fallback_principal: principal,
        store: state.store.clone(),
        hmac_key: Some(hmac_key),
    };
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
            axum::http::Method::PATCH,
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

async fn resolve_gateway_principal(
    state: &GatewayState,
) -> Result<AuthPrincipal, crate::error::ChannelError> {
    let user_id = state.user_id.clone();
    let fallback = AuthPrincipal::new(user_id.clone(), UserRole::Admin);
    let Some(store) = state.store.as_ref() else {
        return Ok(fallback);
    };

    let display_name = format!("Gateway User ({})", user_id);
    let ensured = store
        .ensure_user_account(&user_id, &display_name, UserRole::Admin)
        .await
        .map_err(|err| crate::error::ChannelError::StartupFailed {
            name: "gateway".to_string(),
            reason: format!(
                "Failed to bootstrap gateway principal '{}': {}",
                user_id, err
            ),
        })?;

    Ok(AuthPrincipal::new(ensured.id, ensured.role))
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

// --- Memory handlers ---

#[derive(Deserialize)]
pub(crate) struct TreeQuery {
    #[allow(dead_code)]
    pub(crate) depth: Option<usize>,
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    pub(crate) path: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct ReadQuery {
    pub(crate) path: String,
}

/// Maximum size accepted for a single uploaded file (10 MiB).
pub(crate) const UPLOAD_FILE_SIZE_LIMIT: usize = 10 * 1024 * 1024;
/// Maximum size accepted for backup restore uploads (128 MiB).
pub(crate) const BACKUP_RESTORE_SIZE_LIMIT: usize = 128 * 1024 * 1024;

// --- Matter handlers ---

/// Default workspace path prefix where matter directories live.
pub(crate) use crate::channels::web::handlers::common::*;

// --- Jobs handlers ---

// --- Gateway control plane handlers ---

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
