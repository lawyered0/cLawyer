//! Thread and session operations for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate thread management (user input
//! processing, undo/redo, approval, auth, persistence) from the core loop.

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::Agent;
use crate::agent::compaction::{ContextCompactor, MatterCompactionScope};
use crate::agent::dispatcher::{
    AgenticLoopResult, ToolAuditContext, check_auth_required, execute_chat_tool_standalone,
    parse_auth_result,
};
use crate::agent::session::{PendingApproval, Session, ThreadState};
use crate::agent::submission::SubmissionResult;
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::error::Error;
use crate::llm::ChatMessage;
use crate::tools::ApprovalRequirement;

fn approval_requirement_label(requirement: ApprovalRequirement) -> &'static str {
    match requirement {
        ApprovalRequirement::Never => "never",
        ApprovalRequirement::UnlessAutoApproved => "unless_auto_approved",
        ApprovalRequirement::Always => "always",
    }
}

fn thread_set_matter_id_metadata(thread: &mut crate::agent::session::Thread, matter_id: &str) {
    let Some(matter_id) = crate::legal::policy::sanitize_optional_matter_id(matter_id) else {
        return;
    };

    let mut object = thread.metadata.as_object().cloned().unwrap_or_default();
    object.insert(
        "matter_id".to_string(),
        serde_json::Value::String(matter_id),
    );
    thread.metadata = serde_json::Value::Object(object);
}

fn thread_compaction_scope(
    thread: &crate::agent::session::Thread,
    legal: &crate::config::LegalConfig,
) -> Option<MatterCompactionScope> {
    if !legal.enabled {
        return None;
    }
    let matter_id = crate::legal::policy::matter_id_from_metadata(&thread.metadata)?;
    Some(MatterCompactionScope {
        matter_root: legal.matter_root.clone(),
        matter_id,
    })
}

#[derive(Debug, Clone, Copy)]
struct ThreadCompactionSnapshot {
    updated_at: chrono::DateTime<chrono::Utc>,
    turn_count: usize,
    state: ThreadState,
}

impl ThreadCompactionSnapshot {
    fn capture(thread: &crate::agent::session::Thread) -> Self {
        Self {
            updated_at: thread.updated_at,
            turn_count: thread.turns.len(),
            state: thread.state,
        }
    }
}

fn can_apply_compaction_update(
    thread: &crate::agent::session::Thread,
    snapshot: &ThreadCompactionSnapshot,
) -> bool {
    thread.updated_at == snapshot.updated_at
        && thread.turns.len() == snapshot.turn_count
        && thread.state == snapshot.state
}

fn apply_compaction_if_unchanged(
    thread: &mut crate::agent::session::Thread,
    snapshot: &ThreadCompactionSnapshot,
    compacted_thread: crate::agent::session::Thread,
) -> bool {
    if !can_apply_compaction_update(thread, snapshot) {
        return false;
    }
    *thread = compacted_thread;
    true
}

fn conversation_matter_mismatch_message(bound: &str, active: &str) -> String {
    format!(
        "Thread is bound to matter '{}'. Start a new thread to work in matter '{}'.",
        bound, active
    )
}

fn audit_conversation_matter_mismatch(thread_id: Uuid, bound: &str, active: &str, source: &str) {
    crate::legal::audit::inc_blocked_action();
    crate::legal::audit::record(
        "conversation_matter_mismatch_blocked",
        serde_json::json!({
            "thread_id": thread_id.to_string(),
            "bound_matter": bound,
            "requested_matter": active,
            "source": source,
        }),
    );
}

fn audit_conversation_matter_bound(thread_id: Uuid, active: &str, source: &str) {
    crate::legal::audit::record(
        "conversation_matter_bound",
        serde_json::json!({
            "thread_id": thread_id.to_string(),
            "matter_id": active,
            "source": source,
        }),
    );
}

impl Agent {
    /// Hydrate a historical thread from DB into memory if not already present.
    ///
    /// Called before `resolve_thread` so that the session manager finds the
    /// thread on lookup instead of creating a new one.
    ///
    /// Creates an in-memory thread with the exact UUID the frontend sent,
    /// even when the conversation has zero messages (e.g. a brand-new
    /// assistant thread). Without this, `resolve_thread` would mint a
    /// fresh UUID and all messages would land in the wrong conversation.
    pub(super) async fn maybe_hydrate_thread(
        &self,
        message: &IncomingMessage,
        external_thread_id: &str,
    ) {
        // Only hydrate UUID-shaped thread IDs (web gateway uses UUIDs)
        let thread_uuid = match Uuid::parse_str(external_thread_id) {
            Ok(id) => id,
            Err(_) => return,
        };

        // Check if already in memory
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        {
            let sess = session.lock().await;
            if sess.threads.contains_key(&thread_uuid) {
                return;
            }
        }

        // Load history from DB (may be empty for a newly created thread).
        let mut chat_messages: Vec<ChatMessage> = Vec::new();
        let mut bound_matter: Option<String> = None;
        let msg_count;

        if let Some(store) = self.store() {
            let db_messages = store
                .list_conversation_messages(thread_uuid)
                .await
                .unwrap_or_default();
            msg_count = db_messages.len();
            bound_matter = store
                .get_conversation_matter_id(thread_uuid, &message.user_id)
                .await
                .ok()
                .flatten();
            chat_messages = db_messages
                .iter()
                .filter_map(|m| match m.role.as_str() {
                    "user" => Some(ChatMessage::user(&m.content)),
                    "assistant" => Some(ChatMessage::assistant(&m.content)),
                    _ => None,
                })
                .collect();
        } else {
            msg_count = 0;
        }

        // Create thread with the historical ID and restore messages
        let session_id = {
            let sess = session.lock().await;
            sess.id
        };

        let mut thread = crate::agent::session::Thread::with_id(thread_uuid, session_id);
        if !chat_messages.is_empty() {
            thread.restore_from_messages(chat_messages);
        }
        if let Some(matter_id) = bound_matter.as_deref() {
            thread_set_matter_id_metadata(&mut thread, matter_id);
        }

        // Insert into session and register with session manager
        {
            let mut sess = session.lock().await;
            sess.threads.insert(thread_uuid, thread);
            sess.active_thread = Some(thread_uuid);
            sess.last_active_at = chrono::Utc::now();
        }

        self.session_manager
            .register_thread(
                &message.user_id,
                &message.channel,
                thread_uuid,
                Arc::clone(&session),
            )
            .await;

        tracing::debug!(
            "Hydrated thread {} from DB ({} messages)",
            thread_uuid,
            msg_count
        );
    }

    async fn enforce_thread_matter_scope(
        &self,
        message: &IncomingMessage,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        legal: &crate::config::LegalConfig,
    ) -> Result<(), String> {
        if !legal.enabled {
            return Ok(());
        }

        let active_matter = legal
            .active_matter
            .as_deref()
            .and_then(crate::legal::policy::sanitize_optional_matter_id);

        if let Some(store) = self.store() {
            let ensure_result = store
                .ensure_conversation(thread_id, "gateway", &message.user_id, None)
                .await;
            if let Err(err) = ensure_result {
                tracing::warn!(
                    "Failed to ensure conversation {} before matter-scope enforcement: {}",
                    thread_id,
                    err
                );
            } else {
                match store
                    .get_conversation_matter_id(thread_id, &message.user_id)
                    .await
                {
                    Ok(bound) => {
                        if let Some(ref bound_matter) = bound {
                            // Keep in-memory metadata synchronized for compaction/UI.
                            let mut sess = session.lock().await;
                            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                thread_set_matter_id_metadata(thread, bound_matter);
                            }
                        }

                        match (bound.as_deref(), active_matter.as_deref()) {
                            (Some(bound), Some(active)) if bound != active => {
                                audit_conversation_matter_mismatch(thread_id, bound, active, "db");
                                return Err(conversation_matter_mismatch_message(bound, active));
                            }
                            (None, Some(active)) => {
                                store
                                    .bind_conversation_to_matter(
                                        thread_id,
                                        &message.user_id,
                                        active,
                                    )
                                    .await
                                    .map_err(|err| {
                                        format!(
                                            "Failed to bind thread to active matter '{}': {}",
                                            active, err
                                        )
                                    })?;
                                {
                                    let mut sess = session.lock().await;
                                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                                        thread_set_matter_id_metadata(thread, active);
                                    }
                                }
                                audit_conversation_matter_bound(thread_id, active, "db");
                            }
                            _ => {}
                        }
                        return Ok(());
                    }
                    Err(err) => {
                        tracing::warn!(
                            "Failed to load conversation matter binding for {}: {}",
                            thread_id,
                            err
                        );
                    }
                }
            }
        }

        // No DB (or DB failed): enforce with in-memory thread metadata.
        let mut sess = session.lock().await;
        let Some(thread) = sess.threads.get_mut(&thread_id) else {
            return Ok(());
        };
        let bound = crate::legal::policy::matter_id_from_metadata(&thread.metadata);
        match (bound.as_deref(), active_matter.as_deref()) {
            (Some(bound), Some(active)) if bound != active => {
                audit_conversation_matter_mismatch(thread_id, bound, active, "memory");
                Err(conversation_matter_mismatch_message(bound, active))
            }
            (None, Some(active)) => {
                thread_set_matter_id_metadata(thread, active);
                audit_conversation_matter_bound(thread_id, active, "memory");
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub(super) async fn process_user_input(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        content: &str,
    ) -> Result<SubmissionResult, Error> {
        // First check thread state without holding lock during I/O
        let thread_state = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.state
        };

        // Check thread state
        match thread_state {
            ThreadState::Processing => {
                return Ok(SubmissionResult::error(
                    "Turn in progress. Use /interrupt to cancel.",
                ));
            }
            ThreadState::AwaitingApproval => {
                return Ok(SubmissionResult::error(
                    "Waiting for approval. Use /interrupt to cancel.",
                ));
            }
            ThreadState::Completed => {
                return Ok(SubmissionResult::error(
                    "Thread completed. Use /thread new.",
                ));
            }
            ThreadState::Idle | ThreadState::Interrupted => {
                // Can proceed
            }
        }

        crate::legal::audit::record(
            "prompt_received",
            serde_json::json!({
                "user_id": message.user_id.clone(),
                "thread_id": thread_id.to_string(),
                "channel": message.channel.clone(),
            }),
        );

        // Safety validation for user input
        let validation = self.safety().validate_input(content);
        if !validation.is_valid {
            let details = validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(SubmissionResult::error(format!(
                "Input rejected by safety validation: {}",
                details
            )));
        }

        let violations = self.safety().check_policy(content);
        if violations
            .iter()
            .any(|rule| rule.action == crate::safety::PolicyAction::Block)
        {
            return Ok(SubmissionResult::error("Input rejected by safety policy."));
        }

        // Handle explicit commands (starting with /) directly
        // Everything else goes through the normal agentic loop with tools
        let temp_message = IncomingMessage {
            content: content.to_string(),
            ..message.clone()
        };

        if let Some(intent) = self.router.route_command(&temp_message) {
            // Explicit command like /status, /job, /list - handle directly
            return self.handle_job_or_command(intent, message).await;
        }

        let effective_legal_config = self.effective_legal_config_for(message);

        if let Err(reason) = self
            .enforce_thread_matter_scope(message, &session, thread_id, &effective_legal_config)
            .await
        {
            return Ok(SubmissionResult::error(reason));
        }

        if effective_legal_config.enabled
            && effective_legal_config.require_matter_context
            && effective_legal_config.active_matter.is_none()
            && crate::legal::policy::is_non_trivial_request(content)
        {
            crate::legal::audit::inc_blocked_action();
            crate::legal::audit::record(
                "blocked_missing_matter",
                serde_json::json!({
                    "thread_id": thread_id.to_string(),
                    "reason": "active_matter_required",
                }),
            );
            return Ok(SubmissionResult::error(
                "An active matter is required for legal work. Start cLawyer with --matter <matter_id>.",
            ));
        }

        if effective_legal_config.enabled
            && effective_legal_config.require_matter_context
            && crate::legal::policy::is_non_trivial_request(content)
            && let Some(ws) = self.workspace()
            && let Err(reason) = crate::legal::matter::validate_active_matter_metadata(
                ws.as_ref(),
                &effective_legal_config,
            )
            .await
        {
            crate::legal::audit::inc_blocked_action();
            crate::legal::audit::record(
                "blocked_invalid_matter_metadata",
                serde_json::json!({
                    "thread_id": thread_id.to_string(),
                    "reason": reason,
                }),
            );
            return Ok(SubmissionResult::error(
                "Active matter metadata is incomplete or invalid. Update matters/<matter_id>/matter.yaml before continuing.",
            ));
        }

        if let Some(ws) = self.workspace()
            && let Some(conflict) = crate::legal::matter::detect_conflict_with_store(
                self.store(),
                ws.as_ref(),
                &effective_legal_config,
                content,
            )
            .await
        {
            crate::legal::audit::inc_blocked_action();
            crate::legal::audit::record(
                "conflict_check_hit",
                serde_json::json!({
                    "thread_id": thread_id.to_string(),
                    "conflict": conflict,
                }),
            );
            return Ok(SubmissionResult::error(
                "Potential conflict detected. Review conflict records and acknowledge before continuing.",
            ));
        }

        // Natural language goes through the agentic loop
        // Job tools (create_job, list_jobs, etc.) are in the tool registry

        // Auto-compact if needed BEFORE adding new turn, without holding
        // the session lock across async compaction work.
        let auto_compaction_job = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let messages = thread.messages();
            self.context_monitor
                .suggest_compaction(&messages)
                .map(|strategy| {
                    let pct = self.context_monitor.usage_percent(&messages);
                    (
                        strategy,
                        pct,
                        thread_compaction_scope(thread, &effective_legal_config),
                        ThreadCompactionSnapshot::capture(thread),
                        thread.clone(),
                    )
                })
        };

        if let Some((strategy, pct, scope, snapshot, mut compacted_thread)) = auto_compaction_job {
            tracing::info!("Context at {:.1}% capacity, auto-compacting", pct);

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status(format!("Context at {:.0}% capacity, compacting...", pct)),
                    &message.metadata,
                )
                .await;

            let compactor = ContextCompactor::new(self.llm().clone(), self.safety().clone());
            match compactor
                .compact(
                    &mut compacted_thread,
                    strategy,
                    self.workspace().map(|w| w.as_ref()),
                    scope.as_ref(),
                )
                .await
            {
                Ok(_) => {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                        && !apply_compaction_if_unchanged(thread, &snapshot, compacted_thread)
                    {
                        tracing::debug!(
                            "Skipping auto-compaction apply for thread {}: thread changed",
                            thread_id
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("Auto-compaction failed: {}", e);
                }
            }
        }

        // Create checkpoint before turn
        let (checkpoint_turn, checkpoint_messages) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            (thread.turn_number(), thread.messages())
        };
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        {
            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(
                checkpoint_turn,
                checkpoint_messages,
                format!("Before turn {}", checkpoint_turn),
            );
        }

        // Start the turn and get messages
        let turn_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.start_turn(content);
            thread.messages()
        };

        // Persist user message to DB immediately so it survives crashes
        self.persist_user_message(thread_id, &message.user_id, content)
            .await;

        // Send thinking status
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::Thinking("Processing...".into()),
                &message.metadata,
            )
            .await;

        // Run the agentic tool execution loop
        let result = self
            .run_agentic_loop(message, session.clone(), thread_id, turn_messages)
            .await;

        // Re-acquire lock and check if interrupted
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        if thread.state == ThreadState::Interrupted {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Interrupted".into()),
                    &message.metadata,
                )
                .await;
            return Ok(SubmissionResult::Interrupted);
        }

        // Complete, fail, or request approval
        match result {
            Ok(AgenticLoopResult::Response(response)) => {
                // Hook: TransformResponse — allow hooks to modify or reject the final response
                let mut response = {
                    let event = crate::hooks::HookEvent::ResponseTransform {
                        user_id: message.user_id.clone(),
                        thread_id: thread_id.to_string(),
                        response: response.clone(),
                    };
                    match self.hooks().run(&event).await {
                        Err(crate::hooks::HookError::Rejected { reason }) => {
                            format!("[Response filtered: {}]", reason)
                        }
                        Err(err) => {
                            format!("[Response blocked by hook policy: {}]", err)
                        }
                        Ok(crate::hooks::HookOutcome::Continue {
                            modified: Some(new_response),
                        }) => new_response,
                        _ => response, // fail-open: use original
                    }
                };

                if effective_legal_config.enabled
                    && effective_legal_config.citation_required
                    && !crate::legal::policy::response_has_citation_markers(&response)
                {
                    crate::legal::audit::record(
                        "citation_missing",
                        serde_json::json!({
                            "thread_id": thread_id.to_string(),
                        }),
                    );
                    response.push_str(
                        "\n\n[Draft status: structured citations not detected. Verify sources and supporting authority before relying on this output.]",
                    );
                }

                thread.complete_turn(&response);
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Done".into()),
                        &message.metadata,
                    )
                    .await;

                // Persist assistant response (user message already persisted at turn start)
                self.persist_assistant_response(thread_id, &message.user_id, &response)
                    .await;

                Ok(SubmissionResult::response(response))
            }
            Ok(AgenticLoopResult::NeedApproval { pending }) => {
                // Store pending approval in thread and update state
                let request_id = pending.request_id;
                let tool_name = pending.tool_name.clone();
                let description = pending.description.clone();
                let parameters = pending.parameters.clone();
                thread.await_approval(pending);
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;
                Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                })
            }
            Err(e) => {
                thread.fail_turn(e.to_string());
                // User message already persisted at turn start; nothing else to save
                Ok(SubmissionResult::error(e.to_string()))
            }
        }
    }

    async fn persist_conversation_message(
        &self,
        thread_id: Uuid,
        user_id: &str,
        role: &str,
        content: &str,
        failure_label: &str,
    ) {
        let store = match self.store() {
            Some(s) => Arc::clone(s),
            None => return,
        };

        if let Err(e) = store
            .ensure_conversation(thread_id, "gateway", user_id, None)
            .await
        {
            tracing::warn!("Failed to ensure conversation {}: {}", thread_id, e);
            return;
        }

        if let Err(e) = store
            .add_conversation_message(thread_id, role, content)
            .await
        {
            tracing::warn!("Failed to persist {}: {}", failure_label, e);
        }
    }

    /// Persist the user message to the DB at turn start (before the agentic loop).
    ///
    /// This ensures the user message is durable even if the process crashes
    /// mid-response. Call this right after `thread.start_turn()`.
    pub(super) async fn persist_user_message(
        &self,
        thread_id: Uuid,
        user_id: &str,
        user_input: &str,
    ) {
        self.persist_conversation_message(thread_id, user_id, "user", user_input, "user message")
            .await;
    }

    /// Persist the assistant response to the DB after the agentic loop completes.
    ///
    /// Re-ensures the conversation row exists so that assistant responses are
    /// still persisted even if `persist_user_message` failed transiently at
    /// turn start (e.g. a brief DB blip that resolved before response time).
    pub(super) async fn persist_assistant_response(
        &self,
        thread_id: Uuid,
        user_id: &str,
        response: &str,
    ) {
        self.persist_conversation_message(
            thread_id,
            user_id,
            "assistant",
            response,
            "assistant message",
        )
        .await;
    }

    pub(super) async fn process_undo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let (current_turn, current_messages) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            (thread.turn_number(), thread.messages())
        };

        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let (checkpoint, undo_count) = {
            let mut mgr = undo_mgr.lock().await;
            if !mgr.can_undo() {
                return Ok(SubmissionResult::ok_with_message("Nothing to undo."));
            }
            let checkpoint = mgr.undo(current_turn, current_messages);
            let undo_count = mgr.undo_count();
            (checkpoint, undo_count)
        };

        if let Some(checkpoint) = checkpoint {
            let turn_number = checkpoint.turn_number;
            let messages = checkpoint.messages;
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Undone to turn {}. {} undo(s) remaining.",
                turn_number, undo_count
            )))
        } else {
            Ok(SubmissionResult::error("Undo failed."))
        }
    }

    pub(super) async fn process_redo(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let (current_turn, current_messages) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            (thread.turn_number(), thread.messages())
        };

        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let checkpoint = {
            let mut mgr = undo_mgr.lock().await;
            if !mgr.can_redo() {
                return Ok(SubmissionResult::ok_with_message("Nothing to redo."));
            }
            mgr.redo(current_turn, current_messages)
        };

        if let Some(checkpoint) = checkpoint {
            let turn_number = checkpoint.turn_number;
            let messages = checkpoint.messages;
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Redone to turn {}.",
                turn_number
            )))
        } else {
            Ok(SubmissionResult::error("Redo failed."))
        }
    }

    pub(super) async fn process_interrupt(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let mut sess = session.lock().await;
        let thread = sess
            .threads
            .get_mut(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        match thread.state {
            ThreadState::Processing | ThreadState::AwaitingApproval => {
                thread.interrupt();
                Ok(SubmissionResult::ok_with_message("Interrupted."))
            }
            _ => Ok(SubmissionResult::ok_with_message("Nothing to interrupt.")),
        }
    }

    pub(super) async fn process_compact(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let (usage, strategy, scope, snapshot, mut compacted_thread) = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            let messages = thread.messages();
            let usage = self.context_monitor.usage_percent(&messages);
            let strategy = self
                .context_monitor
                .suggest_compaction(&messages)
                .unwrap_or(
                    crate::agent::context_monitor::CompactionStrategy::Summarize { keep_recent: 5 },
                );
            (
                usage,
                strategy,
                thread_compaction_scope(thread, self.base_legal_config()),
                ThreadCompactionSnapshot::capture(thread),
                thread.clone(),
            )
        };

        let compactor = ContextCompactor::new(self.llm().clone(), self.safety().clone());
        match compactor
            .compact(
                &mut compacted_thread,
                strategy,
                self.workspace().map(|w| w.as_ref()),
                scope.as_ref(),
            )
            .await
        {
            Ok(result) => {
                let apply_result = {
                    let mut sess = session.lock().await;
                    let thread = sess.threads.get_mut(&thread_id).ok_or_else(|| {
                        Error::from(crate::error::JobError::NotFound { id: thread_id })
                    })?;
                    apply_compaction_if_unchanged(thread, &snapshot, compacted_thread)
                };

                if !apply_result {
                    return Ok(SubmissionResult::ok_with_message(
                        "Compaction result skipped because the thread changed during compaction.",
                    ));
                }

                let mut msg = format!(
                    "Compacted: {} turns removed, {} → {} tokens (was {:.1}% full)",
                    result.turns_removed, result.tokens_before, result.tokens_after, usage
                );
                if result.summary_written {
                    msg.push_str(", summary saved to workspace");
                }
                Ok(SubmissionResult::ok_with_message(msg))
            }
            Err(e) => Ok(SubmissionResult::error(format!("Compaction failed: {}", e))),
        }
    }

    pub(super) async fn process_clear(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.turns.clear();
            thread.state = ThreadState::Idle;
        }

        // Clear undo history too
        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        undo_mgr.lock().await.clear();

        Ok(SubmissionResult::ok_with_message("Thread cleared."))
    }

    /// Process an approval or rejection of a pending tool execution.
    pub(super) async fn process_approval(
        &self,
        message: &IncomingMessage,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        request_id: Option<Uuid>,
        approved: bool,
        always: bool,
    ) -> Result<SubmissionResult, Error> {
        let effective_legal_config = self.effective_legal_config_for(message);

        if let Err(reason) = self
            .enforce_thread_matter_scope(message, &session, thread_id, &effective_legal_config)
            .await
        {
            return Ok(SubmissionResult::error(reason));
        }

        // Get pending approval for this thread
        let pending = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            if thread.state != ThreadState::AwaitingApproval {
                return Ok(SubmissionResult::error("No pending approval request."));
            }

            thread.take_pending_approval()
        };

        let pending = match pending {
            Some(p) => p,
            None => return Ok(SubmissionResult::error("No pending approval request.")),
        };

        // Verify request ID if provided
        if let Some(req_id) = request_id
            && req_id != pending.request_id
        {
            // Put it back and return error
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.await_approval(pending);
            }
            return Ok(SubmissionResult::error(
                "Request ID mismatch. Use the correct request ID.",
            ));
        }

        if approved {
            // If always, add to auto-approved set
            if always {
                let legal_forced = self.tools().is_legal_approval_forced_with_legal(
                    &pending.tool_name,
                    Some(&effective_legal_config),
                );
                if !legal_forced {
                    let mut sess = session.lock().await;
                    sess.auto_approve_tool(&pending.tool_name);
                    tracing::info!(
                        "Auto-approved tool '{}' for session {}",
                        pending.tool_name,
                        sess.id
                    );
                }
            }

            crate::legal::audit::record(
                "approval_response",
                serde_json::json!({
                    "approved": true,
                    "always": always,
                    "tool_name": pending.tool_name.clone(),
                }),
            );

            // Reset thread state to processing
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.state = ThreadState::Processing;
                }
            }

            // Execute the approved tool and continue the loop
            let job_ctx =
                JobContext::with_user(&message.user_id, "chat", "Interactive chat session");

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolStarted {
                        name: pending.tool_name.clone(),
                    },
                    &message.metadata,
                )
                .await;

            let tool_result = self
                .execute_chat_tool(
                    &pending.tool_name,
                    &pending.parameters,
                    &job_ctx,
                    Some(ToolAuditContext {
                        thread_id: thread_id.to_string(),
                    }),
                )
                .await;

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::ToolCompleted {
                        name: pending.tool_name.clone(),
                        success: tool_result.is_ok(),
                    },
                    &message.metadata,
                )
                .await;

            if let Ok(ref output) = tool_result
                && !output.is_empty()
            {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::ToolResult {
                            name: pending.tool_name.clone(),
                            preview: output.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
            }

            // Build context including the tool result
            let mut context_messages = pending.context_messages;
            let deferred_tool_calls = pending.deferred_tool_calls;

            // Record result in thread
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id)
                    && let Some(turn) = thread.last_turn_mut()
                {
                    match &tool_result {
                        Ok(output) => {
                            turn.record_tool_result(serde_json::json!(output));
                        }
                        Err(e) => {
                            turn.record_tool_error(e.to_string());
                        }
                    }
                }
            }

            // If tool_auth returned awaiting_token, enter auth mode and
            // return instructions directly (skip agentic loop continuation).
            if let Some((ext_name, instructions)) =
                check_auth_required(&pending.tool_name, &tool_result)
            {
                self.handle_auth_intercept(
                    &session,
                    thread_id,
                    message,
                    &tool_result,
                    ext_name,
                    instructions.clone(),
                )
                .await;
                return Ok(SubmissionResult::response(instructions));
            }

            // Add tool result to context
            let result_content = match tool_result {
                Ok(output) => {
                    let sanitized = self
                        .safety()
                        .sanitize_tool_output(&pending.tool_name, &output);
                    self.safety().wrap_for_llm(
                        &pending.tool_name,
                        &sanitized.content,
                        sanitized.was_modified,
                    )
                }
                Err(e) => format!("Error: {}", e),
            };

            context_messages.push(ChatMessage::tool_result(
                &pending.tool_call_id,
                &pending.tool_name,
                result_content,
            ));

            // Replay deferred tool calls from the same assistant message so
            // every tool_use ID gets a matching tool_result before the next
            // LLM call.
            if !deferred_tool_calls.is_empty() {
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Thinking(format!(
                            "Executing {} deferred tool(s)...",
                            deferred_tool_calls.len()
                        )),
                        &message.metadata,
                    )
                    .await;
            }

            // === Phase 1: Preflight (sequential) ===
            // Walk deferred tools checking approval. Collect runnable
            // tools; stop at the first that needs approval.
            let mut runnable: Vec<crate::llm::ToolCall> = Vec::new();
            let mut approval_needed: Option<(
                usize,
                crate::llm::ToolCall,
                Arc<dyn crate::tools::Tool>,
            )> = None;
            let tool_audit_ctx = ToolAuditContext {
                thread_id: thread_id.to_string(),
            };

            for (idx, tc) in deferred_tool_calls.iter().enumerate() {
                if let Some(tool) = self.tools().get(&tc.name).await {
                    let is_auto_approved = {
                        let sess = session.lock().await;
                        sess.is_tool_auto_approved(&tc.name)
                    };
                    let requirement = tool.requires_approval(&tc.arguments);
                    let decision = self.tools().approval_decision_for_with_legal(
                        &tc.name,
                        requirement,
                        is_auto_approved,
                        Some(&effective_legal_config),
                    );
                    crate::legal::audit::record(
                        "approval_decision",
                        serde_json::json!({
                            "thread_id": thread_id.to_string(),
                            "tool_name": tc.name,
                            "needs_approval": decision.needs_approval,
                            "legal_forced": decision.legal_forced,
                            "auto_approved": is_auto_approved,
                            "requirement": approval_requirement_label(requirement),
                            "source": "deferred_preflight",
                        }),
                    );

                    if decision.needs_approval {
                        crate::legal::audit::inc_approval_required();
                        crate::legal::audit::record(
                            "approval_required",
                            serde_json::json!({
                                "tool_name": tc.name,
                                "legal_forced": decision.legal_forced,
                            }),
                        );
                        approval_needed = Some((idx, tc.clone(), tool));
                        break; // remaining tools stay deferred
                    }
                }

                runnable.push(tc.clone());
            }

            // === Phase 2: Parallel execution ===
            let exec_results: Vec<(crate::llm::ToolCall, Result<String, Error>)> = if runnable.len()
                <= 1
            {
                // Single tool (or none): execute inline
                let mut results = Vec::new();
                for tc in &runnable {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolStarted {
                                name: tc.name.clone(),
                            },
                            &message.metadata,
                        )
                        .await;

                    let result = self
                        .execute_chat_tool(
                            &tc.name,
                            &tc.arguments,
                            &job_ctx,
                            Some(tool_audit_ctx.clone()),
                        )
                        .await;

                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolCompleted {
                                name: tc.name.clone(),
                                success: result.is_ok(),
                            },
                            &message.metadata,
                        )
                        .await;

                    results.push((tc.clone(), result));
                }
                results
            } else {
                // Multiple tools: execute in parallel via JoinSet
                let mut join_set = JoinSet::new();
                let runnable_count = runnable.len();

                for (spawn_idx, tc) in runnable.iter().enumerate() {
                    let tools = self.tools().clone();
                    let safety = self.safety().clone();
                    let channels = self.channels.clone();
                    let job_ctx = job_ctx.clone();
                    let tc = tc.clone();
                    let channel = message.channel.clone();
                    let metadata = message.metadata.clone();
                    let audit_ctx = tool_audit_ctx.clone();

                    join_set.spawn(async move {
                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::ToolStarted {
                                    name: tc.name.clone(),
                                },
                                &metadata,
                            )
                            .await;

                        let result = execute_chat_tool_standalone(
                            &tools,
                            &safety,
                            &tc.name,
                            &tc.arguments,
                            &job_ctx,
                            Some(audit_ctx),
                        )
                        .await;

                        let _ = channels
                            .send_status(
                                &channel,
                                StatusUpdate::ToolCompleted {
                                    name: tc.name.clone(),
                                    success: result.is_ok(),
                                },
                                &metadata,
                            )
                            .await;

                        (spawn_idx, tc, result)
                    });
                }

                // Collect and reorder by original index
                let mut ordered: Vec<Option<(crate::llm::ToolCall, Result<String, Error>)>> =
                    (0..runnable_count).map(|_| None).collect();
                while let Some(join_result) = join_set.join_next().await {
                    match join_result {
                        Ok((idx, tc, result)) => {
                            ordered[idx] = Some((tc, result));
                        }
                        Err(e) => {
                            if e.is_panic() {
                                tracing::error!("Deferred tool execution task panicked: {}", e);
                            } else {
                                tracing::error!("Deferred tool execution task cancelled: {}", e);
                            }
                        }
                    }
                }

                // Fill panicked slots with error results
                ordered
                    .into_iter()
                    .enumerate()
                    .map(|(i, opt)| {
                        opt.unwrap_or_else(|| {
                            let tc = runnable[i].clone();
                            let err: Error = crate::error::ToolError::ExecutionFailed {
                                name: tc.name.clone(),
                                reason: "Task failed during execution".to_string(),
                            }
                            .into();
                            (tc, Err(err))
                        })
                    })
                    .collect()
            };

            // === Phase 3: Post-flight (sequential, in original order) ===
            // Process all results before any conditional return so every
            // tool result is recorded in the session audit trail.
            let mut deferred_auth: Option<String> = None;

            for (tc, deferred_result) in exec_results {
                if let Ok(ref output) = deferred_result
                    && !output.is_empty()
                {
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::ToolResult {
                                name: tc.name.clone(),
                                preview: output.clone(),
                            },
                            &message.metadata,
                        )
                        .await;
                }

                // Record in thread
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id)
                        && let Some(turn) = thread.last_turn_mut()
                    {
                        match &deferred_result {
                            Ok(output) => turn.record_tool_result(serde_json::json!(output)),
                            Err(e) => turn.record_tool_error(e.to_string()),
                        }
                    }
                }

                // Auth detection — defer return until all results are recorded
                if deferred_auth.is_none()
                    && let Some((ext_name, instructions)) =
                        check_auth_required(&tc.name, &deferred_result)
                {
                    self.handle_auth_intercept(
                        &session,
                        thread_id,
                        message,
                        &deferred_result,
                        ext_name,
                        instructions.clone(),
                    )
                    .await;
                    deferred_auth = Some(instructions);
                }

                let deferred_content = match deferred_result {
                    Ok(output) => {
                        let sanitized = self.safety().sanitize_tool_output(&tc.name, &output);
                        self.safety().wrap_for_llm(
                            &tc.name,
                            &sanitized.content,
                            sanitized.was_modified,
                        )
                    }
                    Err(e) => format!("Error: {}", e),
                };

                context_messages.push(ChatMessage::tool_result(&tc.id, &tc.name, deferred_content));
            }

            // Return auth response after all results are recorded
            if let Some(instructions) = deferred_auth {
                return Ok(SubmissionResult::response(instructions));
            }

            // Handle approval if a tool needed it
            if let Some((approval_idx, tc, tool)) = approval_needed {
                let new_pending = PendingApproval {
                    request_id: Uuid::new_v4(),
                    tool_name: tc.name.clone(),
                    parameters: tc.arguments.clone(),
                    description: tool.description().to_string(),
                    tool_call_id: tc.id.clone(),
                    context_messages: context_messages.clone(),
                    deferred_tool_calls: deferred_tool_calls[approval_idx + 1..].to_vec(),
                };

                let request_id = new_pending.request_id;
                let tool_name = new_pending.tool_name.clone();
                let description = new_pending.description.clone();
                let parameters = new_pending.parameters.clone();

                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.await_approval(new_pending);
                    }
                }

                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::Status("Awaiting approval".into()),
                        &message.metadata,
                    )
                    .await;

                return Ok(SubmissionResult::NeedApproval {
                    request_id,
                    tool_name,
                    description,
                    parameters,
                });
            }

            // Continue the agentic loop (a tool was already executed this turn)
            let result = self
                .run_agentic_loop(message, session.clone(), thread_id, context_messages)
                .await;

            // Handle the result
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

            match result {
                Ok(AgenticLoopResult::Response(mut response)) => {
                    if effective_legal_config.enabled
                        && effective_legal_config.citation_required
                        && !crate::legal::policy::response_has_citation_markers(&response)
                    {
                        crate::legal::audit::record(
                            "citation_missing",
                            serde_json::json!({
                                "thread_id": thread_id.to_string(),
                            }),
                        );
                        response.push_str(
                            "\n\n[Draft status: structured citations not detected. Verify sources and supporting authority before relying on this output.]",
                        );
                    }
                    thread.complete_turn(&response);
                    // User message already persisted at turn start; save assistant response
                    self.persist_assistant_response(thread_id, &message.user_id, &response)
                        .await;
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Done".into()),
                            &message.metadata,
                        )
                        .await;
                    Ok(SubmissionResult::response(response))
                }
                Ok(AgenticLoopResult::NeedApproval {
                    pending: new_pending,
                }) => {
                    let request_id = new_pending.request_id;
                    let tool_name = new_pending.tool_name.clone();
                    let description = new_pending.description.clone();
                    let parameters = new_pending.parameters.clone();
                    thread.await_approval(new_pending);
                    let _ = self
                        .channels
                        .send_status(
                            &message.channel,
                            StatusUpdate::Status("Awaiting approval".into()),
                            &message.metadata,
                        )
                        .await;
                    Ok(SubmissionResult::NeedApproval {
                        request_id,
                        tool_name,
                        description,
                        parameters,
                    })
                }
                Err(e) => {
                    thread.fail_turn(e.to_string());
                    // User message already persisted at turn start
                    Ok(SubmissionResult::error(e.to_string()))
                }
            }
        } else {
            crate::legal::audit::record(
                "approval_response",
                serde_json::json!({
                    "approved": false,
                    "always": always,
                    "tool_name": pending.tool_name.clone(),
                }),
            );
            // Rejected - complete the turn with a rejection message and persist
            let rejection = format!(
                "Tool '{}' was rejected. The agent will not execute this tool.\n\n\
                 You can continue the conversation or try a different approach.",
                pending.tool_name
            );
            {
                let mut sess = session.lock().await;
                if let Some(thread) = sess.threads.get_mut(&thread_id) {
                    thread.clear_pending_approval();
                    thread.complete_turn(&rejection);
                    // User message already persisted at turn start; save rejection response
                    self.persist_assistant_response(thread_id, &message.user_id, &rejection)
                        .await;
                }
            }

            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Status("Rejected".into()),
                    &message.metadata,
                )
                .await;

            Ok(SubmissionResult::response(rejection))
        }
    }

    /// Handle an auth-required result from a tool execution.
    ///
    /// Enters auth mode on the thread, completes + persists the turn,
    /// and sends the AuthRequired status to the channel.
    /// Returns the instructions string for the caller to wrap in a response.
    async fn handle_auth_intercept(
        &self,
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
        message: &IncomingMessage,
        tool_result: &Result<String, Error>,
        ext_name: String,
        instructions: String,
    ) {
        let auth_data = parse_auth_result(tool_result);
        {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.enter_auth_mode(ext_name.clone());
                thread.complete_turn(&instructions);
                // User message already persisted at turn start; save auth instructions
                self.persist_assistant_response(thread_id, &message.user_id, &instructions)
                    .await;
            }
        }
        let _ = self
            .channels
            .send_status(
                &message.channel,
                StatusUpdate::AuthRequired {
                    extension_name: ext_name,
                    instructions: Some(instructions.clone()),
                    auth_url: auth_data.auth_url,
                    setup_url: auth_data.setup_url,
                },
                &message.metadata,
            )
            .await;
    }

    /// Handle an auth token submitted while the thread is in auth mode.
    ///
    /// The token goes directly to the extension manager's credential store,
    /// completely bypassing logging, turn creation, history, and compaction.
    pub(super) async fn process_auth_token(
        &self,
        message: &IncomingMessage,
        pending: &crate::agent::session::PendingAuth,
        token: &str,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<Option<String>, Error> {
        let token = token.trim();

        // Clear auth mode regardless of outcome
        {
            let mut sess = session.lock().await;
            if let Some(thread) = sess.threads.get_mut(&thread_id) {
                thread.pending_auth = None;
            }
        }

        let ext_mgr = match self.deps.extension_manager.as_ref() {
            Some(mgr) => mgr,
            None => return Ok(Some("Extension manager not available.".to_string())),
        };

        match ext_mgr.auth(&pending.extension_name, Some(token)).await {
            Ok(result) if result.status == "authenticated" => {
                tracing::info!(
                    "Extension '{}' authenticated via auth mode",
                    pending.extension_name
                );

                // Auto-activate so tools are available immediately after auth
                match ext_mgr.activate(&pending.extension_name).await {
                    Ok(activate_result) => {
                        let tool_count = activate_result.tools_loaded.len();
                        let tool_list = if activate_result.tools_loaded.is_empty() {
                            String::new()
                        } else {
                            format!("\n\nTools: {}", activate_result.tools_loaded.join(", "))
                        };
                        let msg = format!(
                            "{} authenticated and activated ({} tools loaded).{}",
                            pending.extension_name, tool_count, tool_list
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Extension '{}' authenticated but activation failed: {}",
                            pending.extension_name,
                            e
                        );
                        let msg = format!(
                            "{} authenticated successfully, but activation failed: {}. \
                             Try activating manually.",
                            pending.extension_name, e
                        );
                        let _ = self
                            .channels
                            .send_status(
                                &message.channel,
                                StatusUpdate::AuthCompleted {
                                    extension_name: pending.extension_name.clone(),
                                    success: true,
                                    message: msg.clone(),
                                },
                                &message.metadata,
                            )
                            .await;
                        Ok(Some(msg))
                    }
                }
            }
            Ok(result) => {
                // Invalid token, re-enter auth mode
                {
                    let mut sess = session.lock().await;
                    if let Some(thread) = sess.threads.get_mut(&thread_id) {
                        thread.enter_auth_mode(pending.extension_name.clone());
                    }
                }
                let msg = result
                    .instructions
                    .clone()
                    .unwrap_or_else(|| "Invalid token. Please try again.".to_string());
                // Re-emit AuthRequired so web UI re-shows the card
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthRequired {
                            extension_name: pending.extension_name.clone(),
                            instructions: Some(msg.clone()),
                            auth_url: result.auth_url,
                            setup_url: result.setup_url,
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
            Err(e) => {
                let msg = format!(
                    "Authentication failed for {}: {}",
                    pending.extension_name, e
                );
                let _ = self
                    .channels
                    .send_status(
                        &message.channel,
                        StatusUpdate::AuthCompleted {
                            extension_name: pending.extension_name.clone(),
                            success: false,
                            message: msg.clone(),
                        },
                        &message.metadata,
                    )
                    .await;
                Ok(Some(msg))
            }
        }
    }

    pub(super) async fn process_new_thread(
        &self,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;
        let thread = sess.create_thread();
        let thread_id = thread.id;
        Ok(SubmissionResult::ok_with_message(format!(
            "New thread: {}",
            thread_id
        )))
    }

    pub(super) async fn process_switch_thread(
        &self,
        message: &IncomingMessage,
        target_thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let session = self
            .session_manager
            .get_or_create_session(&message.user_id)
            .await;
        let mut sess = session.lock().await;

        if sess.switch_thread(target_thread_id) {
            Ok(SubmissionResult::ok_with_message(format!(
                "Switched to thread {}",
                target_thread_id
            )))
        } else {
            Ok(SubmissionResult::error("Thread not found."))
        }
    }

    pub(super) async fn process_resume(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        checkpoint_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        {
            let sess = session.lock().await;
            let _ = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
        }

        let undo_mgr = self.session_manager.get_undo_manager(thread_id).await;
        let checkpoint = {
            let mut mgr = undo_mgr.lock().await;
            mgr.restore(checkpoint_id)
        };

        if let Some(checkpoint) = checkpoint {
            let description = checkpoint.description;
            let messages = checkpoint.messages;
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.restore_from_messages(messages);
            Ok(SubmissionResult::ok_with_message(format!(
                "Resumed from checkpoint: {}",
                description
            )))
        } else {
            Ok(SubmissionResult::error("Checkpoint not found."))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use tokio::sync::Mutex;

    use super::{ThreadCompactionSnapshot, apply_compaction_if_unchanged};
    use crate::agent::agent_loop::{Agent, AgentDeps};
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::session::{Session, Thread};
    use crate::agent::submission::SubmissionResult;
    use crate::channels::ChannelManager;
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::llm::{
        ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role,
        ToolCompletionRequest, ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::ToolRegistry;

    struct StaticLlmProvider;

    #[async_trait]
    impl LlmProvider for StaticLlmProvider {
        fn model_name(&self) -> &str {
            "static-thread-ops-test"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: vec![],
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }
    }

    fn make_test_agent() -> Agent {
        let legal = crate::config::LegalConfig::resolve(&crate::settings::Settings::default())
            .expect("default legal config should resolve");
        let deps = AgentDeps {
            store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            legal_config: legal,
            hooks: Arc::new(crate::hooks::HookRegistry::new()),
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
        };

        Agent::new(
            AgentConfig {
                name: "thread-ops-test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(30),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations: 25,
                auto_approve_tools: false,
            },
            deps,
            Arc::new(ChannelManager::new()),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        )
    }

    fn seeded_thread(session_id: uuid::Uuid) -> Thread {
        let mut thread = Thread::new(session_id);
        thread.start_turn("turn-1");
        thread.complete_turn("resp-1");
        thread.start_turn("turn-2");
        thread.complete_turn("resp-2");
        thread
    }

    fn message_signature(messages: &[ChatMessage]) -> Vec<(Role, String)> {
        messages
            .iter()
            .map(|message| (message.role, message.content.clone()))
            .collect()
    }

    #[test]
    fn compaction_apply_skips_when_thread_changed() {
        let mut thread = seeded_thread(uuid::Uuid::new_v4());
        let snapshot = ThreadCompactionSnapshot::capture(&thread);
        let mut compacted_thread = thread.clone();
        compacted_thread.truncate_turns(1);

        thread.start_turn("late-turn");
        thread.complete_turn("late-resp");
        let turns_after_change = thread.turns.len();

        let applied = apply_compaction_if_unchanged(&mut thread, &snapshot, compacted_thread);
        assert!(!applied);
        assert_eq!(thread.turns.len(), turns_after_change);
    }

    #[test]
    fn compaction_apply_updates_when_thread_unchanged() {
        let mut thread = seeded_thread(uuid::Uuid::new_v4());
        let snapshot = ThreadCompactionSnapshot::capture(&thread);
        let mut compacted_thread = thread.clone();
        compacted_thread.truncate_turns(1);

        let applied = apply_compaction_if_unchanged(&mut thread, &snapshot, compacted_thread);
        assert!(applied);
        assert_eq!(thread.turns.len(), 1);
    }

    #[tokio::test]
    async fn undo_redo_and_resume_restore_expected_thread_states() {
        let agent = make_test_agent();
        let session = Arc::new(Mutex::new(Session::new("user-thread-ops")));

        let (thread_id, checkpoint_messages) = {
            let mut sess = session.lock().await;
            let thread = sess.create_thread();
            thread.start_turn("turn-1");
            thread.complete_turn("resp-1");
            let checkpoint_messages = thread.messages();
            thread.start_turn("turn-2");
            thread.complete_turn("resp-2");
            (thread.id, checkpoint_messages)
        };

        let undo_mgr = agent.session_manager.get_undo_manager(thread_id).await;
        {
            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(2, checkpoint_messages.clone(), "Before turn 2");
        };

        let undo_result = agent
            .process_undo(Arc::clone(&session), thread_id)
            .await
            .expect("undo should succeed");
        assert!(matches!(undo_result, SubmissionResult::Ok { .. }));
        {
            let sess = session.lock().await;
            let thread = sess.threads.get(&thread_id).expect("thread must exist");
            assert_eq!(
                message_signature(&thread.messages()),
                message_signature(&checkpoint_messages)
            );
        }

        let redo_result = agent
            .process_redo(Arc::clone(&session), thread_id)
            .await
            .expect("redo should succeed");
        assert!(matches!(redo_result, SubmissionResult::Ok { .. }));
        {
            let sess = session.lock().await;
            let thread = sess.threads.get(&thread_id).expect("thread must exist");
            assert_eq!(thread.turns.len(), 2);
        }

        let resume_target_messages = {
            let mut sess = session.lock().await;
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .expect("thread should exist for resume setup");
            let resume_target_messages = thread.messages();
            let resume_turn = thread.turn_number();

            let mut mgr = undo_mgr.lock().await;
            mgr.checkpoint(
                resume_turn,
                resume_target_messages.clone(),
                "Resume target checkpoint",
            );

            thread.start_turn("turn-3");
            thread.complete_turn("resp-3");
            assert_eq!(thread.turns.len(), 3);
            resume_target_messages
        };

        let resume_checkpoint_id = {
            let mgr = undo_mgr.lock().await;
            mgr.list_checkpoints()
                .last()
                .expect("resume checkpoint should exist")
                .id
        };

        let resume_result = agent
            .process_resume(Arc::clone(&session), thread_id, resume_checkpoint_id)
            .await
            .expect("resume should succeed");
        assert!(matches!(resume_result, SubmissionResult::Ok { .. }));

        let sess = session.lock().await;
        let thread = sess.threads.get(&thread_id).expect("thread must exist");
        assert_eq!(
            message_signature(&thread.messages()),
            message_signature(&resume_target_messages)
        );
    }
}
