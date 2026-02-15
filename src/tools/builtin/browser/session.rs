//! Browser session management.
//!
//! Owns the Chrome process lifecycle and per-tab state. Sessions are spawned
//! lazily on first browser action and torn down when dropped.
//!
//! ```text
//! BrowserSession
//! ├── Browser (chromiumoxide, owns Chrome child process)
//! ├── handler_task (JoinHandle polling CDP WebSocket)
//! ├── tabs: HashMap<tab_id, Page>
//! ├── active_tab: current tab id
//! └── element_refs: ElementRefMap (valid until next read_page)
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chromiumoxide::Page;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::accessibility::GetFullAxTreeParams;
use chromiumoxide::cdp::browser_protocol::dom::{GetBoxModelParams, ScrollIntoViewIfNeededParams};
use chromiumoxide::cdp::browser_protocol::input::{
    DispatchMouseEventParams, DispatchMouseEventType, InsertTextParams, MouseButton,
};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::tools::builtin::browser::accessibility::{
    ElementFilter, ElementRefMap, build_page_repr,
};
use crate::tools::builtin::browser::stealth;
use crate::tools::tool::ToolError;

/// Manages a Chrome browser instance and its tabs.
pub struct BrowserSession {
    #[allow(dead_code)] // Used by new_tab() which is reserved for tab management actions
    browser: Browser,
    _handler_task: JoinHandle<()>,
    tabs: HashMap<String, Page>,
    active_tab: String,
    element_refs: Arc<RwLock<ElementRefMap>>,
    #[allow(dead_code)] // Used by new_tab() which is reserved for tab management actions
    stealth_js: String,
}

impl BrowserSession {
    /// Launch a new Chrome browser session.
    ///
    /// Locates Chrome on the system, applies stealth patches, and opens
    /// an initial blank tab.
    pub async fn launch() -> Result<Self, ToolError> {
        let chrome_path = find_chrome().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "Chrome/Chromium not found. Install Chrome or set CHROME_PATH.".to_string(),
            )
        })?;

        // Shared profile so the agent accumulates useful state across sessions
        // (logged-in sessions, dismissed cookie banners, local storage).
        // Delete ~/.ironclaw/browser/profile/ to reset.
        let profile_dir = browser_profile_dir();
        std::fs::create_dir_all(&profile_dir).map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to create browser profile dir: {}", e))
        })?;

        let mut config_builder = BrowserConfig::builder()
            .chrome_executable(&chrome_path)
            .user_data_dir(&profile_dir)
            .window_size(1920, 1080)
            .no_sandbox();

        for arg in stealth::stealth_args() {
            config_builder = config_builder.arg(arg);
        }

        let config = config_builder.build().map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to build browser config: {}", e))
        })?;

        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to launch Chrome: {}", e)))?;

        // The handler must be polled continuously or the CDP connection dies.
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if event.is_err() {
                    tracing::warn!("Browser handler error: {:?}", event);
                    break;
                }
            }
        });

        // Open initial tab.
        let page = browser.new_page("about:blank").await.map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to open initial tab: {}", e))
        })?;

        // Inject stealth JS on every new document load for this page.
        let stealth_js = stealth::stealth_js().to_string();
        page.evaluate_on_new_document(stealth_js.clone())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to inject stealth JS: {}", e))
            })?;

        let tab_id = "tab0".to_string();
        let mut tabs = HashMap::new();
        tabs.insert(tab_id.clone(), page);

        Ok(Self {
            browser,
            _handler_task: handler_task,
            tabs,
            active_tab: tab_id,
            element_refs: Arc::new(RwLock::new(ElementRefMap::new())),
            stealth_js,
        })
    }

    /// Get the active page, or error if session is broken.
    fn active_page(&self) -> Result<&Page, ToolError> {
        self.tabs.get(&self.active_tab).ok_or_else(|| {
            ToolError::ExecutionFailed(format!("No active tab: {}", self.active_tab))
        })
    }

    // --- Navigation ---

    pub async fn navigate(&self, url: &str) -> Result<String, ToolError> {
        let page = self.active_page()?;
        page.goto(url)
            .await
            .map_err(|e| ToolError::ExternalService(format!("Navigation failed: {}", e)))?;

        let title = page
            .get_title()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to get page title: {}", e)))?
            .unwrap_or_default();

        Ok(title)
    }

    pub async fn go_back(&self) -> Result<(), ToolError> {
        let page = self.active_page()?;
        page.evaluate("window.history.back()")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to go back: {}", e)))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }

    pub async fn go_forward(&self) -> Result<(), ToolError> {
        let page = self.active_page()?;
        page.evaluate("window.history.forward()")
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to go forward: {}", e)))?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        Ok(())
    }

    // --- Page reading ---

    /// Build accessibility tree representation and update element refs.
    pub async fn read_page(&self, filter: ElementFilter) -> Result<String, ToolError> {
        let page = self.active_page()?;

        let url = page
            .url()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to get URL: {}", e)))?
            .unwrap_or_else(|| "about:blank".to_string());

        let title = page
            .get_title()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to get title: {}", e)))?
            .unwrap_or_default();

        // Fetch full accessibility tree via CDP.
        let ax_result = page
            .execute(GetFullAxTreeParams::default())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to get accessibility tree: {}", e))
            })?;

        let nodes = ax_result.result.nodes;

        let mut ref_map = self.element_refs.write().await;
        let repr = build_page_repr(&url, &title, &nodes, filter, &mut ref_map);

        Ok(repr)
    }

    /// Extract text content from the page or a CSS selector.
    pub async fn extract_text(&self, selector: Option<&str>) -> Result<String, ToolError> {
        let page = self.active_page()?;

        let js = match selector {
            Some(sel) => {
                let escaped = serde_json::to_string(sel).map_err(|e| {
                    ToolError::InvalidParameters(format!("Invalid selector: {}", e))
                })?;
                format!(
                    "(() => {{ const el = document.querySelector({}); return el ? el.innerText : null; }})()",
                    escaped
                )
            }
            None => "document.body.innerText".to_string(),
        };

        let result: Option<String> = page
            .evaluate(js.as_str())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to extract text: {}", e)))?
            .into_value()
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to deserialize text: {}", e))
            })?;

        Ok(result.unwrap_or_default())
    }

    // --- Interaction ---

    /// Click an element by reference ID (e.g., "e1" or "@e1").
    ///
    /// Uses DOM.scrollIntoViewIfNeeded + DOM.getBoxModel to find the element's
    /// center coordinates, then dispatches mouse press + release at that point.
    pub async fn click_element(&self, ref_id: &str) -> Result<(), ToolError> {
        let page = self.active_page()?;
        let refs = self.element_refs.read().await;

        let elem_ref = refs.get(ref_id).ok_or_else(|| {
            ToolError::InvalidParameters(format!(
                "Unknown element reference '{}'. Call browser with action 'read_page' first.",
                ref_id
            ))
        })?;

        let backend_node_id = elem_ref.backend_node_id;
        drop(refs);

        // Scroll the element into the viewport.
        page.execute(
            ScrollIntoViewIfNeededParams::builder()
                .backend_node_id(backend_node_id)
                .build(),
        )
        .await
        .map_err(|e| {
            ToolError::ExecutionFailed(format!("Failed to scroll element into view: {}", e))
        })?;

        // Get element's bounding box via DOM.getBoxModel.
        let box_result = page
            .execute(
                GetBoxModelParams::builder()
                    .backend_node_id(backend_node_id)
                    .build(),
            )
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to get element box model: {}", e))
            })?;

        // Content quad is [x1,y1, x2,y2, x3,y3, x4,y4]. Center = average of 4 corners.
        let content = box_result.result.model.content.inner();
        if content.len() < 8 {
            return Err(ToolError::ExecutionFailed(
                "Element has no valid bounding box".to_string(),
            ));
        }

        let x = (content[0] + content[2] + content[4] + content[6]) / 4.0;
        let y = (content[1] + content[3] + content[5] + content[7]) / 4.0;

        // Dispatch mouse press + release at center of element.
        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MousePressed)
                .x(x)
                .y(y)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to build mouse event: {}", e))
                })?,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Mouse press failed: {}", e)))?;

        page.execute(
            DispatchMouseEventParams::builder()
                .r#type(DispatchMouseEventType::MouseReleased)
                .x(x)
                .y(y)
                .button(MouseButton::Left)
                .click_count(1)
                .build()
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to build mouse event: {}", e))
                })?,
        )
        .await
        .map_err(|e| ToolError::ExecutionFailed(format!("Mouse release failed: {}", e)))?;

        Ok(())
    }

    /// Type text into an element by reference ID.
    pub async fn type_text(&self, ref_id: &str, text: &str) -> Result<(), ToolError> {
        // First click to focus the element.
        self.click_element(ref_id).await?;

        // Brief delay to let focus settle.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let page = self.active_page()?;

        // Use CDP insertText for reliable IME-style text entry.
        page.execute(InsertTextParams::new(text))
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to type text: {}", e)))?;

        Ok(())
    }

    /// Scroll the page.
    pub async fn scroll(&self, direction: &str, amount: u32) -> Result<(), ToolError> {
        let page = self.active_page()?;

        let (dx, dy) = match direction {
            "up" => (0, -(amount as i32 * 100)),
            "down" => (0, amount as i32 * 100),
            "left" => (-(amount as i32 * 100), 0),
            "right" => (amount as i32 * 100, 0),
            _ => {
                return Err(ToolError::InvalidParameters(format!(
                    "Invalid scroll direction '{}'. Use: up, down, left, right",
                    direction
                )));
            }
        };

        let js = format!("window.scrollBy({}, {})", dx, dy);
        page.evaluate(js.as_str())
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Scroll failed: {}", e)))?;

        Ok(())
    }

    /// Wait for a CSS selector to appear, or a fixed timeout.
    pub async fn wait(&self, selector: Option<&str>, timeout_ms: u64) -> Result<bool, ToolError> {
        let page = self.active_page()?;
        let timeout = std::time::Duration::from_millis(timeout_ms);

        match selector {
            Some(sel) => {
                let poll_interval = std::time::Duration::from_millis(100);
                let start = std::time::Instant::now();
                let escaped = serde_json::to_string(sel).map_err(|e| {
                    ToolError::InvalidParameters(format!("Invalid selector: {}", e))
                })?;

                loop {
                    let js = format!("!!document.querySelector({})", escaped);
                    let found: bool = page
                        .evaluate(js.as_str())
                        .await
                        .map_err(|e| {
                            ToolError::ExecutionFailed(format!("Wait poll failed: {}", e))
                        })?
                        .into_value()
                        .unwrap_or(false);

                    if found {
                        return Ok(true);
                    }

                    if start.elapsed() >= timeout {
                        return Ok(false);
                    }

                    tokio::time::sleep(poll_interval).await;
                }
            }
            None => {
                tokio::time::sleep(timeout).await;
                Ok(true)
            }
        }
    }

    // --- Screenshots ---

    /// Capture a screenshot as base64-encoded PNG.
    pub async fn screenshot(&self, full_page: bool) -> Result<String, ToolError> {
        let page = self.active_page()?;

        let params = ScreenshotParams::builder()
            .format(CaptureScreenshotFormat::Png)
            .full_page(full_page)
            .build();

        let bytes = page
            .screenshot(params)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Screenshot failed: {}", e)))?;

        use base64::Engine;
        Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
    }

    // --- JavaScript ---

    /// Execute arbitrary JavaScript and return the result.
    pub async fn eval_js(&self, expression: &str) -> Result<serde_json::Value, ToolError> {
        let page = self.active_page()?;

        let result = page
            .evaluate(expression)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("JS evaluation failed: {}", e)))?;

        let value: serde_json::Value = result.into_value().unwrap_or(serde_json::Value::Null);

        Ok(value)
    }

    // --- Tab management ---

    /// Open a new tab and make it active.
    #[allow(dead_code)] // Reserved for tab management actions
    pub async fn new_tab(&mut self, url: &str) -> Result<String, ToolError> {
        let page =
            self.browser.new_page(url).await.map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to open new tab: {}", e))
            })?;

        // Inject stealth JS on the new page too.
        page.evaluate_on_new_document(self.stealth_js.clone())
            .await
            .map_err(|e| {
                ToolError::ExecutionFailed(format!("Failed to inject stealth JS on new tab: {}", e))
            })?;

        let tab_id = format!("tab{}", self.tabs.len());
        self.tabs.insert(tab_id.clone(), page);
        self.active_tab = tab_id.clone();

        // Clear element refs since we're on a new page.
        self.element_refs.write().await.reset();

        Ok(tab_id)
    }

    /// List open tabs.
    #[allow(dead_code)] // Reserved for tab management actions
    pub fn list_tabs(&self) -> Vec<String> {
        self.tabs.keys().cloned().collect()
    }

    /// Switch to a different tab.
    #[allow(dead_code)] // Reserved for tab management actions
    pub async fn switch_tab(&mut self, tab_id: &str) -> Result<(), ToolError> {
        if !self.tabs.contains_key(tab_id) {
            return Err(ToolError::InvalidParameters(format!(
                "Unknown tab '{}'. Open tabs: {:?}",
                tab_id,
                self.list_tabs()
            )));
        }

        self.active_tab = tab_id.to_string();
        // Clear element refs when switching tabs.
        self.element_refs.write().await.reset();
        Ok(())
    }

    /// Get current page URL.
    pub async fn current_url(&self) -> Result<String, ToolError> {
        let page = self.active_page()?;
        page.url()
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to get URL: {}", e)))
            .map(|u| u.unwrap_or_else(|| "about:blank".to_string()))
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        tracing::debug!("Browser session dropping, Chrome process will be cleaned up");
    }
}

/// Returns `~/.ironclaw/browser/profile/`.
fn browser_profile_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ironclaw")
        .join("browser")
        .join("profile")
}

/// Search common locations for a Chrome/Chromium binary.
pub fn find_chrome() -> Option<PathBuf> {
    // Environment variable override.
    if let Ok(path) = std::env::var("CHROME_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Some(p);
        }
    }

    let candidates = if cfg!(target_os = "macos") {
        vec![
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Google Chrome Canary.app/Contents/MacOS/Google Chrome Canary",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ]
    } else if cfg!(target_os = "linux") {
        vec![
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ]
    } else {
        // Windows paths.
        vec![
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        ]
    };

    for candidate in candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Some(p);
        }
    }

    which_chrome_in_path()
}

/// Check if chrome/chromium is available in PATH.
fn which_chrome_in_path() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for name in &["google-chrome", "chromium", "chromium-browser", "chrome"] {
        for dir in path_var.split(separator) {
            let candidate = PathBuf::from(dir).join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::tools::builtin::browser::session::find_chrome;

    #[test]
    fn test_find_chrome_returns_path_or_none() {
        let result = find_chrome();
        if let Some(path) = &result {
            assert!(
                path.exists(),
                "find_chrome returned non-existent path: {:?}",
                path
            );
        }
    }
}
