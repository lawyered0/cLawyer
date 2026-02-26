//! End-to-end integration tests for the WebSocket gateway.
//!
//! These tests start a real Axum server on a random port, connect a WebSocket
//! client, and verify the full message flow:
//! - WebSocket upgrade with auth
//! - Ping/pong
//! - Client message → agent msg_tx
//! - Broadcast SSE event → WebSocket client
//! - Connection tracking (counter increment/decrement)
//! - Gateway status endpoint

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use clawyer::channels::IncomingMessage;
use clawyer::channels::web::server::{GatewayState, start_server};
use clawyer::channels::web::sse::SseManager;
use clawyer::channels::web::types::SseEvent;
use clawyer::channels::web::ws::WsConnectionTracker;
use clawyer::db::Database;
#[cfg(feature = "libsql")]
use clawyer::db::libsql::LibSqlBackend;
use clawyer::workspace::Workspace;

const AUTH_TOKEN: &str = "test-token-12345";
const TIMEOUT: Duration = Duration::from_secs(5);

fn legal_config_for_tests() -> clawyer::config::LegalConfig {
    clawyer::config::LegalConfig {
        enabled: true,
        jurisdiction: "us-general".to_string(),
        hardening: clawyer::config::LegalHardeningProfile::MaxLockdown,
        require_matter_context: true,
        citation_required: true,
        matter_root: "matters".to_string(),
        active_matter: None,
        privilege_guard: true,
        conflict_check_enabled: true,
        network: clawyer::config::LegalNetworkConfig {
            deny_by_default: true,
            allowed_domains: Vec::new(),
        },
        audit: clawyer::config::LegalAuditConfig {
            enabled: true,
            path: std::path::PathBuf::from("logs/legal_audit.jsonl"),
            hash_chain: true,
        },
        redaction: clawyer::config::LegalRedactionConfig {
            pii: true,
            phi: true,
            financial: true,
            government_id: true,
        },
    }
}

/// Start a gateway server on a random port and return the bound address + agent
/// message receiver.
async fn start_test_server_with_overrides(
    workspace: Option<Arc<Workspace>>,
    legal_config: Option<clawyer::config::LegalConfig>,
) -> (
    SocketAddr,
    Arc<GatewayState>,
    mpsc::Receiver<IncomingMessage>,
) {
    let (agent_tx, agent_rx) = mpsc::channel(64);

    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(Some(agent_tx)),
        sse: SseManager::new(),
        workspace,
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
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None,
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: clawyer::channels::web::server::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        startup_time: std::time::Instant::now(),
        legal_config,
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state.clone(), AUTH_TOKEN.to_string())
        .await
        .expect("Failed to start test server");

    (bound_addr, state, agent_rx)
}

/// Start a basic gateway server with default legal config and no workspace.
async fn start_test_server() -> (
    SocketAddr,
    Arc<GatewayState>,
    mpsc::Receiver<IncomingMessage>,
) {
    start_test_server_with_overrides(None, None).await
}

#[cfg(feature = "libsql")]
async fn make_test_workspace(user_id: &str) -> Arc<Workspace> {
    let db_path = std::env::temp_dir().join(format!("clawyer-ws-test-{}.db", uuid::Uuid::new_v4()));
    let db: Arc<dyn Database> = Arc::new(
        LibSqlBackend::new_local(&db_path)
            .await
            .expect("local libsql should initialize"),
    );
    db.run_migrations()
        .await
        .expect("libsql migrations should run");
    Arc::new(Workspace::new_with_db(user_id, db))
}

/// Connect a WebSocket client with auth token in query parameter.
async fn connect_ws(
    addr: SocketAddr,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{}/api/chat/ws?token={}", addr, AUTH_TOKEN);
    let mut request = url.into_client_request().unwrap();
    // Server requires an Origin header from localhost to prevent cross-site WS hijacking.
    request.headers_mut().insert(
        "Origin",
        format!("http://127.0.0.1:{}", addr.port()).parse().unwrap(),
    );
    let (stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect WebSocket");
    stream
}

/// Read the next text frame from the WebSocket, with a timeout.
async fn recv_text(
    stream: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> String {
    let msg = timeout(TIMEOUT, stream.next())
        .await
        .expect("Timed out waiting for WS message")
        .expect("Stream ended")
        .expect("WS error");
    match msg {
        Message::Text(text) => text.to_string(),
        other => panic!("Expected Text frame, got {:?}", other),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::test]
async fn test_ws_ping_pong() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send ping
    let ping = r#"{"type":"ping"}"#;
    ws.send(Message::Text(ping.into())).await.unwrap();

    // Expect pong
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "pong");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_message_reaches_agent() {
    let (addr, _state, mut agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send a chat message
    let msg = r#"{"type":"message","content":"hello from ws","thread_id":"t42"}"#;
    ws.send(Message::Text(msg.into())).await.unwrap();

    // Verify it arrives on the agent's msg_tx
    let incoming = timeout(TIMEOUT, agent_rx.recv())
        .await
        .expect("Timed out waiting for agent message")
        .expect("Agent channel closed");

    assert_eq!(incoming.content, "hello from ws");
    assert_eq!(incoming.thread_id.as_deref(), Some("t42"));
    assert_eq!(incoming.channel, "gateway");
    assert_eq!(incoming.user_id, "test-user");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_broadcast_event_received() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Give the connection a moment to fully establish
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast an SSE event (simulates agent sending a response)
    state.sse.broadcast(SseEvent::Response {
        content: "agent says hi".to_string(),
        thread_id: "t1".to_string(),
    });

    // The WS client should receive it
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "response");
    assert_eq!(parsed["data"]["content"], "agent says hi");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_thinking_event() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    state.sse.broadcast(SseEvent::Thinking {
        message: "analyzing...".to_string(),
        thread_id: None,
    });

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "event");
    assert_eq!(parsed["event_type"], "thinking");
    assert_eq!(parsed["data"]["message"], "analyzing...");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_connection_tracking() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let tracker = state.ws_tracker.as_ref().unwrap();

    assert_eq!(tracker.connection_count(), 0);

    // Connect first client
    let ws1 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Connect second client
    let ws2 = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(tracker.connection_count(), 2);

    // Disconnect first
    drop(ws1);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 1);

    // Disconnect second
    drop(ws2);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(tracker.connection_count(), 0);
}

#[tokio::test]
async fn test_ws_invalid_message_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send invalid JSON
    ws.send(Message::Text("not json".into())).await.unwrap();

    // Should get an error message back
    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");
    assert!(
        parsed["message"]
            .as_str()
            .unwrap()
            .contains("Invalid message")
    );

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_ws_unknown_type_returns_error() {
    let (addr, _state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;

    // Send valid JSON but unknown message type
    ws.send(Message::Text(r#"{"type":"foobar"}"#.into()))
        .await
        .unwrap();

    let text = recv_text(&mut ws).await;
    let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed["type"], "error");

    ws.close(None).await.unwrap();
}

#[tokio::test]
async fn test_gateway_status_endpoint() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Connect a WS client
    let _ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Hit the status endpoint
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/api/gateway/status", addr))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("Failed to fetch status");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ws_connections"], 1);
    assert!(body["total_connections"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn test_root_response_has_csp_header() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/", addr))
        .send()
        .await
        .expect("Failed to fetch root page");

    assert_eq!(resp.status(), 200);

    let csp = resp
        .headers()
        .get(reqwest::header::CONTENT_SECURITY_POLICY)
        .expect("missing Content-Security-Policy header")
        .to_str()
        .expect("invalid CSP header value");

    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("script-src 'self' https://cdn.jsdelivr.net"));
    assert!(csp.contains("object-src 'none'"));
    assert!(csp.contains("frame-ancestors 'none'"));

    let script_src = csp
        .split(';')
        .map(str::trim)
        .find(|directive| directive.starts_with("script-src"))
        .expect("missing script-src directive");
    assert!(
        !script_src.contains("'unsafe-inline'"),
        "script-src unexpectedly allows 'unsafe-inline': {}",
        script_src
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn gateway_legal_audit_endpoint_requires_auth_and_returns_data() {
    let dir = tempfile::tempdir().expect("tempdir");
    let logs_dir = dir.path().join("logs");
    std::fs::create_dir_all(&logs_dir).expect("create logs dir");
    let audit_path = logs_dir.join("legal_audit.jsonl");
    std::fs::write(
        &audit_path,
        r#"{"ts":"2026-02-25T12:00:00Z","event_type":"prompt_received","details":{"thread_id":"t1"},"metrics":{"blocked_actions":0,"approval_required":0,"redaction_events":0}}"#,
    )
    .expect("write audit fixture");

    let mut legal = legal_config_for_tests();
    legal.enabled = true;
    legal.audit.enabled = true;
    legal.audit.path = audit_path;

    let (addr, _state, _agent_rx) = start_test_server_with_overrides(None, Some(legal)).await;
    let client = reqwest::Client::new();
    let url = format!("http://{}/api/legal/audit?limit=10", addr);

    let unauth = client.get(&url).send().await.expect("unauth request");
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("auth request");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let body: serde_json::Value = resp.json().await.expect("audit response json");
    let events = body["events"].as_array().expect("events array");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event_type"], "prompt_received");
    assert_eq!(body["total"], 1);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn gateway_matters_conflict_check_endpoint_happy_path() {
    let workspace = make_test_workspace("test-user").await;
    workspace
        .write(
            "conflicts.json",
            r#"[{"name":"Acme Corp","aliases":["Acme","Acme Corporation"]}]"#,
        )
        .await
        .expect("seed conflicts");

    let mut legal = legal_config_for_tests();
    legal.enabled = true;
    legal.require_matter_context = false;
    legal.conflict_check_enabled = true;

    let (addr, _state, _agent_rx) =
        start_test_server_with_overrides(Some(workspace), Some(legal)).await;
    let client = reqwest::Client::new();
    let url = format!("http://{}/api/matters/conflicts/check", addr);

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .json(&serde_json::json!({
            "text": "Please review exposure for Acme Corp in this contract dispute."
        }))
        .send()
        .await
        .expect("conflict check response");
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let body: serde_json::Value = resp.json().await.expect("conflict response json");
    assert_eq!(body["matched"], true);
    assert_eq!(body["conflict"], "Acme Corp");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn gateway_matter_template_apply_flow_end_to_end() {
    let workspace = make_test_workspace("test-user").await;
    workspace
        .write(
            "matters/demo/matter.yaml",
            "matter_id: demo\nclient: Demo Client\nteam:\n  - Lead Counsel\nconfidentiality: attorney-client-privileged\nadversaries:\n  - Other Party\nretention: follow-firm-policy\n",
        )
        .await
        .expect("seed matter metadata");
    workspace
        .write(
            "matters/demo/templates/research_memo.md",
            "# Research Memo\n\n## Issue\n\n## Analysis\n",
        )
        .await
        .expect("seed template");

    let mut legal = legal_config_for_tests();
    legal.enabled = true;
    let (addr, _state, _agent_rx) =
        start_test_server_with_overrides(Some(Arc::clone(&workspace)), Some(legal)).await;
    let client = reqwest::Client::new();

    let list_resp = client
        .get(format!("http://{}/api/matters/demo/templates", addr))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("templates list response");
    assert_eq!(list_resp.status(), reqwest::StatusCode::OK);
    let list_body: serde_json::Value = list_resp.json().await.expect("templates list json");
    assert_eq!(list_body["matter_id"], "demo");
    let templates = list_body["templates"].as_array().expect("templates array");
    assert!(
        templates.iter().any(|v| v["name"] == "research_memo.md"),
        "expected research_memo.md in templates list: {templates:?}"
    );

    let apply_resp = client
        .post(format!("http://{}/api/matters/demo/templates/apply", addr))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .json(&serde_json::json!({ "template_name": "research_memo.md" }))
        .send()
        .await
        .expect("apply response");
    assert_eq!(apply_resp.status(), reqwest::StatusCode::CREATED);
    let apply_body: serde_json::Value = apply_resp.json().await.expect("apply json");
    let created_path = apply_body["path"].as_str().expect("created path");
    assert!(
        created_path.starts_with("matters/demo/drafts/research_memo-"),
        "unexpected created path: {created_path}"
    );

    let created_doc = workspace
        .read(created_path)
        .await
        .expect("created draft should exist");
    assert!(created_doc.content.contains("# Research Memo"));

    let docs_resp = client
        .get(format!(
            "http://{}/api/matters/demo/documents?include_templates=false",
            addr
        ))
        .header("Authorization", format!("Bearer {}", AUTH_TOKEN))
        .send()
        .await
        .expect("documents list response");
    assert_eq!(docs_resp.status(), reqwest::StatusCode::OK);
    let docs_body: serde_json::Value = docs_resp.json().await.expect("documents list json");
    let docs = docs_body["documents"].as_array().expect("documents array");
    assert!(
        docs.iter()
            .any(|v| v["path"].as_str() == Some(created_path) && v["is_dir"] == false),
        "expected created draft in documents list: {docs:?}"
    );
}

#[tokio::test]
async fn test_ws_no_auth_rejected() {
    let (addr, _state, _agent_rx) = start_test_server().await;

    // Try to connect without auth token
    let url = format!("ws://{}/api/chat/ws", addr);
    let request = url.into_client_request().unwrap();
    let result = tokio_tungstenite::connect_async(request).await;

    // Should fail (401 from auth middleware before WS upgrade)
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ws_multiple_events_in_sequence() {
    let (addr, state, _agent_rx) = start_test_server().await;
    let mut ws = connect_ws(addr).await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Broadcast multiple events rapidly
    state.sse.broadcast(SseEvent::Thinking {
        message: "step 1".to_string(),
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::ToolStarted {
        name: "shell".to_string(),
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::ToolCompleted {
        name: "shell".to_string(),
        success: true,
        thread_id: None,
    });
    state.sse.broadcast(SseEvent::Response {
        content: "done".to_string(),
        thread_id: "t1".to_string(),
    });

    // Receive all 4 in order
    let t1 = recv_text(&mut ws).await;
    let t2 = recv_text(&mut ws).await;
    let t3 = recv_text(&mut ws).await;
    let t4 = recv_text(&mut ws).await;

    let p1: serde_json::Value = serde_json::from_str(&t1).unwrap();
    let p2: serde_json::Value = serde_json::from_str(&t2).unwrap();
    let p3: serde_json::Value = serde_json::from_str(&t3).unwrap();
    let p4: serde_json::Value = serde_json::from_str(&t4).unwrap();

    assert_eq!(p1["event_type"], "thinking");
    assert_eq!(p2["event_type"], "tool_started");
    assert_eq!(p3["event_type"], "tool_completed");
    assert_eq!(p4["event_type"], "response");

    ws.close(None).await.unwrap();
}
