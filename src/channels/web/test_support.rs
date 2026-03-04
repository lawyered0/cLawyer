//! Shared test helpers for web gateway modules.

use std::sync::Arc;

use async_trait::async_trait;
use rust_decimal::Decimal;

use crate::channels::web::sse::SseManager;
use crate::channels::web::state::{GatewayState, RateLimiter};

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
