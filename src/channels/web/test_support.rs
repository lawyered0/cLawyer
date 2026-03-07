//! Shared test helpers for web gateway modules.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::channels::web::auth::{AuthPrincipal, RequestPrincipal};
use crate::channels::web::sse::SseManager;
use crate::channels::web::state::{GatewayState, RateLimiter};
use crate::db::UserRole;

pub(crate) struct TestLlmProvider {
    pub(crate) model: String,
    pub(crate) content: String,
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
        _request: crate::llm::CompletionRequest,
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

pub(crate) fn minimal_test_gateway_state(
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

pub(crate) fn assert_no_inline_event_handlers(asset_name: &str, content: &str) {
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

// ── Per-test gateway state builders ──────────────────────────────────────────

/// Returns the default legal config (what the binary uses when no overrides are set).
pub(crate) fn test_legal_config() -> crate::config::LegalConfig {
    crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
        .expect("default legal config should resolve")
}

/// Returns a [`RequestPrincipal`] acting as the gateway owner (`test-user`).
///
/// All existing server tests operate as the matter owner, so this grants
/// full access through the RBAC guard via the short-circuit owner path.
pub(crate) fn owner_principal() -> RequestPrincipal {
    RequestPrincipal(AuthPrincipal::new("test-user", UserRole::Admin))
}

/// Build a [`GatewayState`] backed by a real database store, workspace, and
/// explicit legal config. Feature-gated to `libsql` because the test DB
/// helper (`crate::testing::test_db`) only exists with that feature.
#[cfg(feature = "libsql")]
pub(crate) fn test_gateway_state_with_store_workspace_and_legal(
    store: Arc<dyn crate::db::Database>,
    workspace: Arc<crate::workspace::Workspace>,
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
        ws_tracker: Some(Arc::new(crate::channels::web::ws::WsConnectionTracker::new())),
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

/// Like [`test_gateway_state_with_store_workspace_and_legal`] but uses the
/// default legal config.
#[cfg(feature = "libsql")]
pub(crate) fn test_gateway_state_with_store_and_workspace(
    store: Arc<dyn crate::db::Database>,
    workspace: Arc<crate::workspace::Workspace>,
) -> Arc<GatewayState> {
    test_gateway_state_with_store_workspace_and_legal(store, workspace, test_legal_config())
}

/// Like [`test_gateway_state_with_store_and_workspace`] but also wires up a
/// [`SessionManager`] so that chat handler tests can read/write thread state.
#[cfg(feature = "libsql")]
pub(crate) fn test_gateway_state_with_store_workspace_and_chat(
    store: Arc<dyn crate::db::Database>,
    workspace: Arc<crate::workspace::Workspace>,
) -> Arc<GatewayState> {
    Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: Some(workspace),
        session_manager: Some(Arc::new(crate::agent::SessionManager::new())),
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: Some(store),
        job_manager: None,
        prompt_queue: None,
        user_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(crate::channels::web::ws::WsConnectionTracker::new())),
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

/// Build a [`GatewayState`] with a custom `user_id`. Useful for RBAC tests that
/// need to simulate requests from a user who is *not* the matter owner.
#[cfg(feature = "libsql")]
pub(crate) fn test_gateway_state_for_user(
    store: Arc<dyn crate::db::Database>,
    workspace: Arc<crate::workspace::Workspace>,
    user_id: &str,
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
        user_id: user_id.to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(crate::channels::web::ws::WsConnectionTracker::new())),
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

/// Seed a minimal matter directory structure in `workspace` for handler tests.
///
/// Creates `matters/{matter_id}/matter.yaml` plus two templates and a notes file.
#[cfg(feature = "libsql")]
pub(crate) async fn seed_valid_matter(workspace: &crate::workspace::Workspace, matter_id: &str) {
    let metadata = format!(
        "matter_id: {matter_id}\nclient: Demo Client\nteam:\n  - Lead Counsel\n\
         confidentiality: attorney-client-privileged\nadversaries:\n  - Example Co\n\
         retention: follow-firm-policy\n"
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
