//! Headless browser tool for web interaction.
//!
//! A single `BrowserTool` that dispatches actions via a tagged enum,
//! keeping the tool registry clean (one tool, not ten). The LLM sends
//! an `action` field to pick the operation:
//!
//! ```json
//! { "action": "navigate", "url": "https://example.com" }
//! { "action": "click", "ref": "@e3" }
//! { "action": "type", "ref": "@e1", "text": "hello" }
//! { "action": "read_page" }
//! { "action": "screenshot" }
//! ```
//!
//! Element references (`@e1`, `@e2`, ...) are assigned by `read_page`
//! and remain valid until the next `read_page` call.

pub mod accessibility;
pub mod session;
pub mod stealth;

use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::tools::builtin::browser::accessibility::ElementFilter;
use crate::tools::builtin::browser::session::BrowserSession;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// Actions the LLM can request from the browser tool.
///
/// Uses serde tagged enum: the JSON `"action"` field selects the variant,
/// remaining fields are variant-specific parameters.
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum BrowserAction {
    /// Navigate to a URL.
    Navigate { url: String },
    /// Go back in browser history.
    Back,
    /// Go forward in browser history.
    Forward,
    /// Read the page's accessibility tree (assigns element refs).
    ReadPage {
        /// "interactive" (default) or "all"
        filter: Option<String>,
    },
    /// Click an element by reference ID.
    Click {
        /// Element reference like "@e1" or "e1".
        #[serde(alias = "ref")]
        ref_id: String,
    },
    /// Type text into an element by reference ID.
    Type {
        /// Element reference like "@e1" or "e1".
        #[serde(alias = "ref")]
        ref_id: String,
        text: String,
    },
    /// Scroll the page.
    Scroll {
        /// "up", "down", "left", "right"
        direction: String,
        /// Number of scroll steps (default 3).
        amount: Option<u32>,
    },
    /// Capture a screenshot (returns base64 PNG).
    Screenshot {
        /// Capture full scrollable page (default false).
        full_page: Option<bool>,
    },
    /// Extract text content from the page or a CSS selector.
    Extract {
        /// Optional CSS selector. If omitted, extracts all body text.
        selector: Option<String>,
    },
    /// Wait for a CSS selector to appear or a fixed delay.
    Wait {
        /// CSS selector to wait for. If omitted, just sleeps.
        selector: Option<String>,
        /// Timeout in milliseconds (default 5000).
        timeout_ms: Option<u64>,
    },
    /// Execute JavaScript (requires user approval).
    EvalJs { expression: String },
}

/// Headless browser tool for navigating web pages, interacting with
/// elements, and extracting content.
///
/// Uses Chrome/Chromium via the DevTools Protocol. The browser is launched
/// lazily on first use and includes basic anti-detection patches.
///
/// ## Workflow
///
/// 1. `navigate` to a URL
/// 2. `read_page` to get the accessibility tree with element refs
/// 3. `click` / `type` using the refs
/// 4. `extract` or `screenshot` to get results
///
/// Element refs (`@e1`, `@e2`) are valid until the next `read_page`.
pub struct BrowserTool {
    /// Lazily initialized browser session. RwLock because `execute` takes `&self`.
    session: RwLock<Option<BrowserSession>>,
}

impl BrowserTool {
    pub fn new() -> Self {
        Self {
            session: RwLock::new(None),
        }
    }

    /// Ensure the browser session is initialized, launching Chrome if needed.
    async fn ensure_session(&self) -> Result<(), ToolError> {
        let needs_launch = self.session.read().await.is_none();
        if needs_launch {
            let new_session = BrowserSession::launch().await?;
            let mut guard = self.session.write().await;
            if guard.is_none() {
                *guard = Some(new_session);
            }
        }
        Ok(())
    }
}

impl Default for BrowserTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Control a headless web browser. Navigate pages, read content, click elements, type text, \
         take screenshots. Use 'read_page' to get an accessibility tree with element references \
         (@e1, @e2...), then use those refs for 'click' and 'type' actions.\n\n\
         Actions: navigate, back, forward, read_page, click, type, scroll, screenshot, extract, \
         wait, eval_js"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": [
                        "navigate", "back", "forward", "read_page", "click",
                        "type", "scroll", "screenshot", "extract", "wait", "eval_js"
                    ],
                    "description": "The browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'navigate' action)"
                },
                "ref_id": {
                    "type": "string",
                    "description": "Element reference like '@e1' (for 'click' and 'type' actions)"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for 'type' action)"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction (for 'scroll' action)"
                },
                "amount": {
                    "type": "integer",
                    "description": "Scroll steps, default 3 (for 'scroll' action)"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture full scrollable page (for 'screenshot' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector (for 'extract' and 'wait' actions)"
                },
                "timeout_ms": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (for 'wait' action, default 5000)"
                },
                "filter": {
                    "type": "string",
                    "enum": ["interactive", "all"],
                    "description": "Element filter for 'read_page' (default: interactive)"
                },
                "expression": {
                    "type": "string",
                    "description": "JavaScript expression (for 'eval_js' action)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let action: BrowserAction = serde_json::from_value(params)
            .map_err(|e| ToolError::InvalidParameters(format!("Invalid browser action: {}", e)))?;

        // Launch browser on first use.
        self.ensure_session().await?;

        match action {
            BrowserAction::Navigate { url } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let title = session.navigate(&url).await?;
                let current_url = session.current_url().await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "url": current_url,
                        "title": title,
                        "status": "navigated"
                    }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Back => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                session.go_back().await?;
                let url = session.current_url().await?;

                Ok(ToolOutput::success(
                    serde_json::json!({ "url": url, "status": "navigated_back" }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Forward => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                session.go_forward().await?;
                let url = session.current_url().await?;

                Ok(ToolOutput::success(
                    serde_json::json!({ "url": url, "status": "navigated_forward" }),
                    start.elapsed(),
                ))
            }

            BrowserAction::ReadPage { filter } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let element_filter = ElementFilter::from_str_opt(filter.as_deref());
                let repr = session.read_page(element_filter).await?;

                Ok(ToolOutput::text(repr, start.elapsed()))
            }

            BrowserAction::Click { ref_id } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                session.click_element(&ref_id).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({ "status": "clicked", "ref": ref_id }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Type { ref_id, text } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                session.type_text(&ref_id, &text).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "typed",
                        "ref": ref_id,
                        "length": text.len()
                    }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Scroll { direction, amount } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let steps = amount.unwrap_or(3);
                session.scroll(&direction, steps).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "status": "scrolled",
                        "direction": direction,
                        "amount": steps
                    }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Screenshot { full_page } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let b64 = session.screenshot(full_page.unwrap_or(false)).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "format": "png",
                        "encoding": "base64",
                        "data": b64,
                        "full_page": full_page.unwrap_or(false)
                    }),
                    start.elapsed(),
                ))
            }

            BrowserAction::Extract { selector } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let text = session.extract_text(selector.as_deref()).await?;

                // Truncate very long text to avoid blowing up context.
                let truncated = if text.len() > 32_000 {
                    format!(
                        "{}...\n\n[truncated, {} total chars]",
                        &text[..32_000],
                        text.len()
                    )
                } else {
                    text.clone()
                };

                Ok(ToolOutput::text(&truncated, start.elapsed()).with_raw(text))
            }

            BrowserAction::Wait {
                selector,
                timeout_ms,
            } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let timeout = timeout_ms.unwrap_or(5000);
                let found = session.wait(selector.as_deref(), timeout).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({
                        "found": found,
                        "selector": selector,
                        "timeout_ms": timeout
                    }),
                    start.elapsed(),
                ))
            }

            BrowserAction::EvalJs { expression } => {
                let session = self.session.read().await;
                let session = session.as_ref().ok_or_else(|| {
                    ToolError::ExecutionFailed("Browser session not initialized".to_string())
                })?;

                let result = session.eval_js(&expression).await?;

                Ok(ToolOutput::success(
                    serde_json::json!({ "result": result }),
                    start.elapsed(),
                ))
            }
        }
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn requires_sanitization(&self) -> bool {
        true // Page content is untrusted external data
    }

    fn requires_approval(&self) -> bool {
        true // Browser navigates to external sites, executes JS
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::builtin::browser::BrowserTool;
    use crate::tools::tool::Tool;

    #[test]
    fn test_browser_tool_metadata() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
        assert!(tool.requires_approval());
        assert!(tool.requires_sanitization());
    }

    #[test]
    fn test_schema_has_action_enum() {
        let tool = BrowserTool::new();
        let schema = tool.parameters_schema();

        let action_prop = schema.get("properties").and_then(|p| p.get("action"));
        assert!(action_prop.is_some());

        let action_enum = action_prop.and_then(|a| a.get("enum"));
        assert!(action_enum.is_some());

        let actions: Vec<&str> = action_enum
            .and_then(|e| e.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        assert!(actions.contains(&"navigate"));
        assert!(actions.contains(&"click"));
        assert!(actions.contains(&"type"));
        assert!(actions.contains(&"read_page"));
        assert!(actions.contains(&"screenshot"));
        assert!(actions.contains(&"eval_js"));
    }

    #[test]
    fn test_action_deserialization() {
        use super::BrowserAction;

        // Navigate
        let action: BrowserAction = serde_json::from_value(
            serde_json::json!({"action": "navigate", "url": "https://x.com"}),
        )
        .unwrap();
        assert!(matches!(action, BrowserAction::Navigate { url } if url == "https://x.com"));

        // Click with "ref" alias
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({"action": "click", "ref": "@e1"})).unwrap();
        assert!(matches!(action, BrowserAction::Click { ref_id } if ref_id == "@e1"));

        // Click with "ref_id"
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({"action": "click", "ref_id": "e2"})).unwrap();
        assert!(matches!(action, BrowserAction::Click { ref_id } if ref_id == "e2"));

        // Type
        let action: BrowserAction = serde_json::from_value(
            serde_json::json!({"action": "type", "ref": "@e1", "text": "hello"}),
        )
        .unwrap();
        assert!(
            matches!(action, BrowserAction::Type { ref_id, text } if ref_id == "@e1" && text == "hello")
        );

        // ReadPage with default filter
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({"action": "read_page"})).unwrap();
        assert!(matches!(action, BrowserAction::ReadPage { filter: None }));

        // Screenshot
        let action: BrowserAction =
            serde_json::from_value(serde_json::json!({"action": "screenshot", "full_page": true}))
                .unwrap();
        assert!(matches!(
            action,
            BrowserAction::Screenshot {
                full_page: Some(true)
            }
        ));

        // Invalid action
        let result: Result<BrowserAction, _> =
            serde_json::from_value(serde_json::json!({"action": "fly_to_moon"}));
        assert!(result.is_err());
    }
}
