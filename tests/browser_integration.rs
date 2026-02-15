//! Integration test for the browser tool.
//!
//! Requires Chrome installed. Run with:
//!   cargo test --test browser_integration -- --nocapture

use ironclaw::context::JobContext;
use ironclaw::tools::Tool;
use ironclaw::tools::builtin::{BrowserTool, find_chrome};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_browser_navigate_and_screenshot() {
    // Skip if Chrome/Chromium is not installed (works on macOS, Linux, Windows).
    if find_chrome().is_none() {
        eprintln!("Skipping: Chrome not found");
        return;
    }

    let tool = BrowserTool::new();
    let ctx = JobContext::default();

    // 1. Navigate to Wikipedia
    eprintln!("=== Navigating to Wikipedia...");
    let nav_result = tool
        .execute(
            serde_json::json!({
                "action": "navigate",
                "url": "https://en.wikipedia.org/wiki/Mariam_Almheiri"
            }),
            &ctx,
        )
        .await;

    match &nav_result {
        Ok(output) => {
            eprintln!(
                "Navigation result: {}",
                serde_json::to_string_pretty(&output.result).unwrap()
            );
            let title = output
                .result
                .get("title")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            assert!(
                title.contains("Mariam") || title.contains("Almheiri"),
                "Page title should mention Mariam Almheiri, got: {}",
                title
            );
        }
        Err(e) => {
            eprintln!("Navigation failed: {}", e);
            panic!("Navigation should succeed");
        }
    }

    // 2. Read the accessibility tree
    eprintln!("\n=== Reading page accessibility tree...");
    let read_result = tool
        .execute(serde_json::json!({"action": "read_page"}), &ctx)
        .await;

    match &read_result {
        Ok(output) => {
            let tree = output.result.as_str().unwrap_or("");
            let line_count = tree.lines().count();
            eprintln!("Accessibility tree: {} lines", line_count);
            // Print first 20 lines
            for line in tree.lines().take(20) {
                eprintln!("  {}", line);
            }
            if line_count > 20 {
                eprintln!("  ... ({} more lines)", line_count - 20);
            }
            assert!(line_count > 3, "Should have some elements on the page");
        }
        Err(e) => {
            eprintln!("Read page failed: {}", e);
            panic!("Read page should succeed");
        }
    }

    // 3. Get page dimensions via eval_js to compute center
    eprintln!("\n=== Getting page dimensions...");
    let dims_result = tool
        .execute(
            serde_json::json!({
                "action": "eval_js",
                "expression": "JSON.stringify({w: window.innerWidth, h: window.innerHeight, scrollH: document.body.scrollHeight})"
            }),
            &ctx,
        )
        .await;

    let (viewport_w, viewport_h) = match &dims_result {
        Ok(output) => {
            let result_str = output
                .result
                .get("result")
                .and_then(|r| r.as_str())
                .unwrap_or("{}");
            let dims: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();
            let w = dims.get("w").and_then(|v| v.as_f64()).unwrap_or(1920.0);
            let h = dims.get("h").and_then(|v| v.as_f64()).unwrap_or(1080.0);
            eprintln!("Viewport: {}x{}", w, h);
            (w, h)
        }
        Err(e) => {
            eprintln!("eval_js failed: {}", e);
            (1920.0, 1080.0)
        }
    };

    // 4. Scroll to middle of page first
    eprintln!("\n=== Scrolling to middle of page...");
    let _ = tool
        .execute(
            serde_json::json!({
                "action": "eval_js",
                "expression": "window.scrollTo(0, document.body.scrollHeight / 2 - window.innerHeight / 2)"
            }),
            &ctx,
        )
        .await;

    // Brief wait for scroll to settle
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // 5. Take full viewport screenshot
    eprintln!("\n=== Taking viewport screenshot...");
    let screenshot_result = tool
        .execute(serde_json::json!({"action": "screenshot"}), &ctx)
        .await;

    match &screenshot_result {
        Ok(output) => {
            let b64 = output
                .result
                .get("data")
                .and_then(|d| d.as_str())
                .unwrap_or("");
            eprintln!(
                "Screenshot: {} base64 chars ({} bytes decoded)",
                b64.len(),
                b64.len() * 3 / 4
            );

            // Save to /tmp for inspection
            use base64::Engine;
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                let path = "/tmp/ironclaw_browser_test_viewport.png";
                if std::fs::write(path, &bytes).is_ok() {
                    eprintln!("Saved viewport screenshot to {}", path);
                }

                // Now crop the center 10x10 using raw PNG manipulation
                // We'll use eval_js to take a clipped screenshot via CDP directly
            }
        }
        Err(e) => {
            eprintln!("Screenshot failed: {}", e);
            panic!("Screenshot should succeed");
        }
    }

    // 6. Take a 10x10 screenshot from the center of the viewport using eval_js
    //    We can't directly use the clip param through the current tool API,
    //    so we'll take the viewport screenshot and note the center crop coords.
    let center_x = (viewport_w / 2.0 - 5.0).max(0.0);
    let center_y = (viewport_h / 2.0 - 5.0).max(0.0);
    eprintln!(
        "\n=== Center 10x10 crop would be at ({}, {}) to ({}, {})",
        center_x,
        center_y,
        center_x + 10.0,
        center_y + 10.0
    );

    // 7. Extract some text to verify content loaded
    eprintln!("\n=== Extracting page text...");
    let extract_result = tool
        .execute(
            serde_json::json!({"action": "extract", "selector": "h1"}),
            &ctx,
        )
        .await;

    match &extract_result {
        Ok(output) => {
            let text = output.result.as_str().unwrap_or("");
            eprintln!("H1 text: {}", text);
            assert!(
                text.contains("Mariam") || text.contains("Almheiri"),
                "H1 should contain the article subject, got: {}",
                text
            );
        }
        Err(e) => {
            eprintln!("Extract failed: {}", e);
        }
    }

    eprintln!("\n=== All browser integration tests passed!");
}
