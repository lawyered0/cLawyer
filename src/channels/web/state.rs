//! Shared web gateway runtime state types.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::{mpsc, oneshot};

use crate::agent::SessionManager;
use crate::channels::IncomingMessage;
use crate::channels::web::log_layer::LogBroadcaster;
use crate::channels::web::sse::SseManager;
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
    /// Snapshot of runtime facts used for compliance scoring.
    pub runtime_facts: crate::compliance::ComplianceRuntimeFacts,
}
