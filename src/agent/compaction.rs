//! Context compaction for preserving and summarizing conversation history.
//!
//! When the context window approaches its limit, compaction:
//! 1. Summarizes old turns
//! 2. Writes the summary to the workspace daily log
//! 3. Trims the context to keep only recent turns

use std::sync::Arc;

use chrono::Utc;

use crate::agent::context_monitor::{CompactionStrategy, ContextBreakdown};
use crate::agent::session::Thread;
use crate::error::Error;
use crate::llm::{ChatMessage, CompletionRequest, LlmProvider, Reasoning};
use crate::safety::SafetyLayer;
use crate::workspace::Workspace;

/// Result of a compaction operation.
#[derive(Debug)]
pub struct CompactionResult {
    /// Number of turns removed.
    pub turns_removed: usize,
    /// Tokens before compaction.
    pub tokens_before: usize,
    /// Tokens after compaction.
    pub tokens_after: usize,
    /// Whether a summary was written to workspace.
    pub summary_written: bool,
    /// The generated summary (if any).
    pub summary: Option<String>,
}

/// Optional matter-bound destination for compaction artifacts.
#[derive(Debug, Clone)]
pub struct MatterCompactionScope {
    pub matter_root: String,
    pub matter_id: String,
}

/// Compacts conversation context to stay within limits.
pub struct ContextCompactor {
    llm: Arc<dyn LlmProvider>,
    safety: Arc<SafetyLayer>,
}

impl ContextCompactor {
    /// Create a new context compactor.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self { llm, safety }
    }

    /// Compact a thread's context using the given strategy.
    pub async fn compact(
        &self,
        thread: &mut Thread,
        strategy: CompactionStrategy,
        workspace: Option<&Workspace>,
        matter_scope: Option<&MatterCompactionScope>,
    ) -> Result<CompactionResult, Error> {
        let messages = thread.messages();
        let tokens_before = ContextBreakdown::analyze(&messages).total_tokens;

        let result = match strategy {
            CompactionStrategy::Summarize { keep_recent } => {
                self.compact_with_summary(thread, keep_recent, workspace, matter_scope)
                    .await?
            }
            CompactionStrategy::Truncate { keep_recent } => {
                self.compact_truncate(thread, keep_recent)
            }
            CompactionStrategy::MoveToWorkspace => {
                self.compact_to_workspace(thread, workspace, matter_scope)
                    .await?
            }
        };

        let messages_after = thread.messages();
        let tokens_after = ContextBreakdown::analyze(&messages_after).total_tokens;

        Ok(CompactionResult {
            turns_removed: result.turns_removed,
            tokens_before,
            tokens_after,
            summary_written: result.summary_written,
            summary: result.summary,
        })
    }

    /// Compact by summarizing old turns.
    async fn compact_with_summary(
        &self,
        thread: &mut Thread,
        keep_recent: usize,
        workspace: Option<&Workspace>,
        matter_scope: Option<&MatterCompactionScope>,
    ) -> Result<CompactionPartial, Error> {
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        // Get turns to summarize
        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Build messages for summarization
        let mut to_summarize = Vec::new();
        for turn in old_turns {
            to_summarize.push(ChatMessage::user(&turn.user_input));
            if let Some(ref response) = turn.response {
                to_summarize.push(ChatMessage::assistant(response));
            }
        }

        // Generate summary
        let summary = self
            .generate_summary(&to_summarize, matter_scope.is_some())
            .await?;

        // Write to workspace if available
        let summary_written = if let Some(ws) = workspace {
            match self
                .write_summary_to_workspace(ws, &summary, matter_scope)
                .await
            {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!(
                        "Compaction summary write failed (turns will still be truncated): {}",
                        e
                    );
                    false
                }
            }
        } else {
            false
        };

        // Truncate thread
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written,
            summary: Some(summary),
        })
    }

    /// Compact by simple truncation (no summary).
    fn compact_truncate(&self, thread: &mut Thread, keep_recent: usize) -> CompactionPartial {
        let turns_before = thread.turns.len();
        thread.truncate_turns(keep_recent);
        let turns_removed = turns_before - thread.turns.len();

        CompactionPartial {
            turns_removed,
            summary_written: false,
            summary: None,
        }
    }

    /// Move context to workspace without summarization.
    async fn compact_to_workspace(
        &self,
        thread: &mut Thread,
        workspace: Option<&Workspace>,
        matter_scope: Option<&MatterCompactionScope>,
    ) -> Result<CompactionPartial, Error> {
        let Some(ws) = workspace else {
            // Fall back to truncation if no workspace
            return Ok(self.compact_truncate(thread, 5));
        };

        // Keep more turns when moving to workspace (we have a backup)
        let keep_recent = 10;
        if thread.turns.len() <= keep_recent {
            return Ok(CompactionPartial::empty());
        }

        let turns_to_remove = thread.turns.len() - keep_recent;
        let old_turns = &thread.turns[..turns_to_remove];

        // Format turns for storage
        let content = format_turns_for_storage(old_turns);

        // Write to workspace
        let written = match self
            .write_context_to_workspace(ws, &content, matter_scope)
            .await
        {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(
                    "Compaction context write failed (turns will still be truncated): {}",
                    e
                );
                false
            }
        };

        // Truncate
        thread.truncate_turns(keep_recent);

        Ok(CompactionPartial {
            turns_removed: turns_to_remove,
            summary_written: written,
            summary: None,
        })
    }

    /// Generate a summary of messages using the LLM.
    async fn generate_summary(
        &self,
        messages: &[ChatMessage],
        legal_mode: bool,
    ) -> Result<String, Error> {
        let prompt = if legal_mode {
            ChatMessage::system(
                r#"Summarize the following legal matter session in markdown.

Use exactly these sections:
- New Facts Learned
- Documents Reviewed and Key Points
- Decisions Made
- Next Steps

Requirements:
- Keep statements evidence-grounded and concise.
- If a section has no support in the transcript, write "insufficient evidence"."#,
            )
        } else {
            ChatMessage::system(
                r#"Summarize the following conversation concisely. Focus on:
- Key decisions made
- Important information exchanged
- Actions taken
- Outcomes achieved

Be brief but capture all important details. Use bullet points."#,
            )
        };

        let mut request_messages = vec![prompt];

        // Add a user message with the conversation to summarize
        let formatted = messages
            .iter()
            .map(|m| {
                let role_str = match m.role {
                    crate::llm::Role::User => "User",
                    crate::llm::Role::Assistant => "Assistant",
                    crate::llm::Role::System => "System",
                    crate::llm::Role::Tool => {
                        return format!(
                            "Tool {}: {}",
                            m.name.as_deref().unwrap_or("unknown"),
                            m.content
                        );
                    }
                };
                format!("{}: {}", role_str, m.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        request_messages.push(ChatMessage::user(format!(
            "Please summarize this conversation:\n\n{}",
            formatted
        )));

        let request = CompletionRequest::new(request_messages)
            .with_max_tokens(1024)
            .with_temperature(0.3);

        let reasoning = Reasoning::new(self.llm.clone(), self.safety.clone());
        let (text, _) = reasoning.complete(request).await?;
        Ok(text)
    }

    /// Write a summary to the workspace daily log.
    async fn write_summary_to_workspace(
        &self,
        workspace: &Workspace,
        summary: &str,
        matter_scope: Option<&MatterCompactionScope>,
    ) -> Result<(), Error> {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let entry = format!(
            "\n## Context Summary ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            summary
        );
        let target = workspace_compaction_path(&date, matter_scope);

        workspace.append(&target, &entry).await?;
        Ok(())
    }

    /// Write full context to workspace for archival.
    async fn write_context_to_workspace(
        &self,
        workspace: &Workspace,
        content: &str,
        matter_scope: Option<&MatterCompactionScope>,
    ) -> Result<(), Error> {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let entry = format!(
            "\n## Archived Context ({})\n\n{}\n",
            Utc::now().format("%H:%M UTC"),
            content
        );
        let target = workspace_compaction_path(&date, matter_scope);

        workspace.append(&target, &entry).await?;
        Ok(())
    }
}

/// Partial result during compaction (internal).
struct CompactionPartial {
    turns_removed: usize,
    summary_written: bool,
    summary: Option<String>,
}

impl CompactionPartial {
    fn empty() -> Self {
        Self {
            turns_removed: 0,
            summary_written: false,
            summary: None,
        }
    }
}

/// Format turns for storage in workspace.
fn format_turns_for_storage(turns: &[crate::agent::session::Turn]) -> String {
    turns
        .iter()
        .map(|turn| {
            let mut s = format!("**Turn {}**\n", turn.turn_number + 1);
            s.push_str(&format!("User: {}\n", turn.user_input));
            if let Some(ref response) = turn.response {
                s.push_str(&format!("Agent: {}\n", response));
            }
            if !turn.tool_calls.is_empty() {
                s.push_str("Tools: ");
                let tools: Vec<_> = turn.tool_calls.iter().map(|t| t.name.as_str()).collect();
                s.push_str(&tools.join(", "));
                s.push('\n');
            }
            s
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn workspace_compaction_path(date: &str, scope: Option<&MatterCompactionScope>) -> String {
    if let Some(scope) = scope {
        let matter_root = scope.matter_root.trim_matches('/');
        let matter_id = crate::legal::policy::sanitize_matter_id(&scope.matter_id);
        if !matter_root.is_empty() && !matter_id.is_empty() {
            return format!("{matter_root}/{matter_id}/sessions/{date}.md");
        }
    }
    format!("daily/{date}.md")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Thread;
    use std::sync::Arc;
    use uuid::Uuid;

    #[test]
    fn test_format_turns() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("Hello");
        thread.complete_turn("Hi there");
        thread.start_turn("How are you?");
        thread.complete_turn("I'm good!");

        let formatted = format_turns_for_storage(&thread.turns);
        assert!(formatted.contains("Turn 1"));
        assert!(formatted.contains("Hello"));
        assert!(formatted.contains("Turn 2"));
    }

    #[test]
    fn test_compaction_partial_empty() {
        let partial = CompactionPartial::empty();
        assert_eq!(partial.turns_removed, 0);
        assert!(!partial.summary_written);
    }

    #[test]
    fn compaction_path_uses_matter_scope_when_available() {
        let scope = MatterCompactionScope {
            matter_root: "matters".to_string(),
            matter_id: "Acme v. Foo".to_string(),
        };
        let path = workspace_compaction_path("2026-03-01", Some(&scope));
        assert_eq!(path, "matters/acme-v--foo/sessions/2026-03-01.md");
    }

    #[test]
    fn compaction_path_falls_back_to_daily_without_scope() {
        let path = workspace_compaction_path("2026-03-01", None);
        assert_eq!(path, "daily/2026-03-01.md");
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn compaction_writes_to_matter_sessions_path_with_libsql_workspace() {
        use crate::agent::context_monitor::CompactionStrategy;
        use crate::config::SafetyConfig;
        use crate::safety::SafetyLayer;
        use crate::testing::{StubLlm, test_db};
        use crate::workspace::Workspace;

        let (db, _tmp) = test_db().await;
        let workspace = Workspace::new_with_db("test-user", Arc::clone(&db));

        let mut thread = Thread::new(Uuid::new_v4());
        thread.start_turn("First user turn");
        thread.complete_turn("First assistant response");
        thread.start_turn("Second user turn");
        thread.complete_turn("Second assistant response");

        let compactor = ContextCompactor::new(
            Arc::new(StubLlm::new("Matter-scoped summary")),
            Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
            })),
        );
        let scope = MatterCompactionScope {
            matter_root: "matters".to_string(),
            matter_id: "Demo Matter".to_string(),
        };

        let result = compactor
            .compact(
                &mut thread,
                CompactionStrategy::Summarize { keep_recent: 1 },
                Some(&workspace),
                Some(&scope),
            )
            .await
            .expect("compaction should succeed");
        assert!(
            result.summary_written,
            "summary should be written to workspace"
        );

        let entries = workspace
            .list("matters/demo-matter/sessions")
            .await
            .expect("matter sessions directory should exist");
        assert!(
            !entries.is_empty(),
            "expected at least one sessions artifact under matter scope"
        );
        let summary_entry = entries
            .iter()
            .find(|entry| !entry.is_directory)
            .expect("expected a summary file in sessions directory");
        assert!(
            summary_entry
                .path
                .starts_with("matters/demo-matter/sessions/"),
            "unexpected summary path: {}",
            summary_entry.path
        );

        let summary_file = workspace
            .read(&summary_entry.path)
            .await
            .expect("summary file should be readable");
        assert!(
            summary_file.content.contains("Matter-scoped summary"),
            "summary content should come from compactor output"
        );
    }
}
