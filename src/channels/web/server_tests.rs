use super::*;
use std::sync::Arc;

use crate::agent::SessionManager;
use crate::channels::web::handlers::{
    chat::{
        chat_approval_handler, chat_history_handler, chat_new_thread_handler, chat_send_handler,
        chat_threads_handler,
    },
    legal::{
        compliance_letter_handler, compliance_status_handler, legal_audit_list_handler,
        legal_court_rules_handler,
    },
    matters::{
        conflicts::{
            matters_conflict_check_handler, matters_conflicts_check_handler,
            matters_conflicts_reindex_handler,
        },
        core::{
            matter_deadlines_compute_handler, matter_deadlines_create_handler,
            matter_deadlines_delete_handler, matter_deadlines_handler, matters_active_get_handler,
            matters_active_set_handler, matters_create_handler, matters_list_handler,
        },
        documents::{
            documents_generate_handler, matter_dashboard_handler, matter_documents_handler,
            matter_filing_package_handler, matter_template_apply_handler, matter_templates_handler,
        },
        finance::{
            invoices_finalize_handler, invoices_payment_handler, invoices_save_handler,
            invoices_void_handler, matter_expenses_create_handler, matter_expenses_list_handler,
            matter_invoices_list_handler, matter_time_create_handler, matter_time_delete_handler,
            matter_time_list_handler, matter_time_summary_handler, matter_trust_deposit_handler,
            matter_trust_ledger_handler,
        },
        work::{
            matter_notes_create_handler, matter_notes_list_handler, matter_tasks_create_handler,
            matter_tasks_list_handler,
        },
    },
    memory::memory_write_handler,
};
use crate::channels::web::sse::SseManager;
use crate::channels::web::test_support::{
    TestLlmProvider, assert_no_inline_event_handlers, minimal_test_gateway_state,
};
use crate::db::ConflictDecision;
use crate::workspace::Workspace;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::Utc;
use regex::Regex;
use uuid::Uuid;

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
            conflict_decision: None,
            conflict_note: None,
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
            conflict_decision: None,
            conflict_note: None,
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
            conflict_decision: None,
            conflict_note: None,
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
async fn matters_active_set_requires_conflict_decision_when_hits_exist() {
    let (db, _tmp) = crate::testing::test_db().await;
    db.seed_matter_parties("existing-matter", "Demo Client", &[], None)
        .await
        .expect("seed existing conflict party");
    let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
    seed_valid_matter(workspace.as_ref(), "demo").await;
    let state =
        test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

    let err = matters_active_set_handler(
        State(Arc::clone(&state)),
        Json(SetActiveMatterRequest {
            matter_id: Some("demo".to_string()),
            conflict_decision: None,
            conflict_note: None,
        }),
    )
    .await
    .expect_err("missing conflict decision should block active set");

    assert_eq!(err.0, StatusCode::CONFLICT);
    let body: serde_json::Value =
        serde_json::from_str(&err.1).expect("conflict body should be json");
    assert_eq!(body["conflict_required"], true);
    assert!(body["hits"].as_array().is_some_and(|hits| !hits.is_empty()));

    let stored = state
        .store
        .as_ref()
        .expect("store")
        .get_setting("test-user", MATTER_ACTIVE_SETTING)
        .await
        .expect("read setting");
    assert!(stored.is_none(), "active matter should remain unset");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn matters_active_set_records_decision_and_reuses_latest_clearance() {
    let _audit_lock = crate::legal::audit::lock_test_event_scenario().await;
    crate::legal::audit::clear_test_events();

    let (db, _tmp) = crate::testing::test_db().await;
    db.seed_matter_parties("existing-matter", "Demo Client", &[], None)
        .await
        .expect("seed existing conflict party");
    let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
    seed_valid_matter(workspace.as_ref(), "demo").await;
    let state =
        test_gateway_state_with_store_and_workspace(Arc::clone(&db), Arc::clone(&workspace));

    let status = matters_active_set_handler(
        State(Arc::clone(&state)),
        Json(SetActiveMatterRequest {
            matter_id: Some("demo".to_string()),
            conflict_decision: Some(ConflictDecision::Waived),
            conflict_note: Some("Waived after attorney review".to_string()),
        }),
    )
    .await
    .expect("waived decision should allow active matter set");
    assert_eq!(status, StatusCode::NO_CONTENT);

    let clearance = state
        .store
        .as_ref()
        .expect("store")
        .latest_conflict_clearance("demo")
        .await
        .expect("latest conflict clearance query should succeed")
        .expect("clearance should be recorded");
    assert_eq!(clearance.decision, ConflictDecision::Waived);
    assert_eq!(clearance.hit_count, 1);

    let status = matters_active_set_handler(
        State(Arc::clone(&state)),
        Json(SetActiveMatterRequest {
            matter_id: Some("demo".to_string()),
            conflict_decision: None,
            conflict_note: None,
        }),
    )
    .await
    .expect("existing matching clearance should allow active set");
    assert_eq!(status, StatusCode::NO_CONTENT);

    let events = crate::legal::audit::test_events_snapshot();
    assert!(events.iter().any(|event| {
        event.event_type == "conflict_clearance_decision"
            && event.details["source"] == "active_set_flow"
            && event.details["decision"] == "waived"
    }));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn matters_active_set_ignores_conflict_hits_for_same_matter() {
    let (db, _tmp) = crate::testing::test_db().await;
    db.seed_matter_parties("demo", "Demo Client", &[], None)
        .await
        .expect("seed same-matter party row");
    let workspace = Arc::new(Workspace::new_with_db("test-user", Arc::clone(&db)));
    seed_valid_matter(workspace.as_ref(), "demo").await;
    let state = test_gateway_state_with_store_and_workspace(db, workspace);

    let status = matters_active_set_handler(
        State(state),
        Json(SetActiveMatterRequest {
            matter_id: Some("demo".to_string()),
            conflict_decision: None,
            conflict_note: None,
        }),
    )
    .await
    .expect("same-matter hit should not require conflict decision");
    assert_eq!(status, StatusCode::NO_CONTENT);
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
            conflict_decision: None,
            conflict_note: None,
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
        alias_hits
            .iter()
            .any(|hit| { hit.matter_id == crate::legal::matter::GLOBAL_CONFLICT_GRAPH_MATTER_ID })
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
        event.event_type == "conflict_clearance_decision" && event.details["decision"] == "waived"
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
                && entry.details.get("source").and_then(|v| v.as_str()) == Some("manual_text_check")
                && entry.details.get("conflict").and_then(|v| v.as_str()) == Some("Alpha Holdings")
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
    let entries = list_matters_root_entries(Err(crate::error::WorkspaceError::DocumentNotFound {
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
    let _ = invoices_finalize_handler(State(Arc::clone(&state)), Path(created.invoice.id.clone()))
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

    let Json(summary) = matter_time_summary_handler(State(Arc::clone(&state)), Path("demo".into()))
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
        crate::legal::matter::detect_conflict(workspace.as_ref(), &legal, "Alpha Holdings").await;
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
        crate::legal::matter::detect_conflict(workspace.as_ref(), &legal, "Beta Partners").await;
    assert_eq!(second.as_deref(), Some("Beta Partners"));
    assert_eq!(
        crate::legal::matter::conflict_cache_refresh_count_for_tests(),
        2
    );
}
