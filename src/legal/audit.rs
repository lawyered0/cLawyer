use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::LegalAuditConfig;

#[derive(Debug, Default, Clone, Serialize)]
struct SecurityMetrics {
    blocked_actions: u64,
    approval_required: u64,
    redaction_events: u64,
}

#[derive(Debug, Serialize)]
struct AuditEvent<'a> {
    ts: String,
    event_type: &'a str,
    details: serde_json::Value,
    metrics: SecurityMetrics,
    #[serde(skip_serializing_if = "Option::is_none")]
    prev_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    hash: Option<String>,
}

struct AuditLogger {
    path: PathBuf,
    hash_chain: bool,
    state: Mutex<Option<String>>,
    metrics: Mutex<SecurityMetrics>,
}

impl AuditLogger {
    fn new(path: PathBuf, hash_chain: bool) -> Self {
        Self {
            path,
            hash_chain,
            state: Mutex::new(None),
            metrics: Mutex::new(SecurityMetrics::default()),
        }
    }

    fn bump_metric<F>(&self, update: F)
    where
        F: FnOnce(&mut SecurityMetrics),
    {
        if let Ok(mut metrics) = self.metrics.lock() {
            update(&mut metrics);
        }
    }

    fn write(&self, event_type: &str, details: serde_json::Value) {
        // Keep metrics + hash-chain state locked through append so the
        // serialized metrics snapshot is atomic with the written event.
        let metrics_guard = match self.metrics.lock() {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Legal audit metrics lock poisoned: {}", e);
                return;
            }
        };

        let mut state = match self.state.lock() {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Legal audit state lock poisoned: {}", e);
                return;
            }
        };
        let metrics = metrics_guard.clone();

        let prev_hash = state.clone();
        let mut event = AuditEvent {
            ts: Utc::now().to_rfc3339(),
            event_type,
            details,
            metrics,
            prev_hash,
            hash: None,
        };

        if self.hash_chain {
            let to_hash = match serde_json::to_string(&event) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to serialize audit event for hashing: {}", e);
                    return;
                }
            };
            let mut hasher = Sha256::new();
            hasher.update(to_hash.as_bytes());
            let hash = format!("{:x}", hasher.finalize());
            event.hash = Some(hash.clone());
            *state = Some(hash);
        }

        let line = match serde_json::to_string(&event) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to serialize legal audit event: {}", e);
                return;
            }
        };

        if let Some(parent) = self.path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!("Failed to create legal audit log dir {:?}: {}", parent, e);
            return;
        }

        // SECURITY: create files as owner-read/write (0o600). For pre-existing
        // files, fail closed if permissions are broader than 0o600.
        let mut open_opts = OpenOptions::new();
        open_opts.create(true).append(true);
        #[cfg(unix)]
        open_opts.mode(0o600);
        match open_opts.open(&self.path) {
            Ok(mut f) => {
                #[cfg(unix)]
                {
                    let mode = match f.metadata() {
                        Ok(meta) => meta.permissions().mode() & 0o777,
                        Err(e) => {
                            tracing::warn!(
                                "Failed to read permissions for legal audit log {:?}: {}",
                                self.path,
                                e
                            );
                            return;
                        }
                    };
                    if mode != 0o600 {
                        tracing::warn!(
                            "Refusing to write legal audit event; insecure mode {:o} on {:?} (expected 600)",
                            mode,
                            self.path
                        );
                        return;
                    }
                }
                if let Err(e) = writeln!(f, "{line}") {
                    tracing::warn!("Failed to append legal audit event: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to open legal audit log {:?}: {}", self.path, e);
            }
        }
    }
}

static LOGGER: OnceLock<AuditLogger> = OnceLock::new();
#[cfg(test)]
static TEST_EVENTS: OnceLock<Mutex<Vec<TestAuditEvent>>> = OnceLock::new();

#[cfg(test)]
#[derive(Debug, Clone)]
pub(crate) struct TestAuditEvent {
    pub event_type: String,
    pub details: serde_json::Value,
}

/// Initialize the legal audit logger.
pub fn init(config: &LegalAuditConfig) {
    if !config.enabled {
        return;
    }

    let _ = LOGGER.set(AuditLogger::new(config.path.clone(), config.hash_chain));
}

/// Log a legal audit event.
pub fn record(event_type: &str, details: serde_json::Value) {
    #[cfg(test)]
    push_test_event(event_type, &details);
    if let Some(logger) = LOGGER.get() {
        logger.write(event_type, details);
    }
}

/// Increment the blocked-action counter.
pub fn inc_blocked_action() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.blocked_actions += 1);
    }
}

/// Increment the approval-required counter.
pub fn inc_approval_required() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.approval_required += 1);
    }
}

/// Increment the redaction-events counter.
pub fn inc_redaction_event() {
    if let Some(logger) = LOGGER.get() {
        logger.bump_metric(|m| m.redaction_events += 1);
    }
}

/// Returns true if audit logging is active.
pub fn enabled() -> bool {
    LOGGER.get().is_some()
}

#[cfg(test)]
fn push_test_event(event_type: &str, details: &serde_json::Value) {
    let events = TEST_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut lock) = events.lock() {
        lock.push(TestAuditEvent {
            event_type: event_type.to_string(),
            details: details.clone(),
        });
    }
}

#[cfg(test)]
pub(crate) fn clear_test_events() {
    if let Some(events) = TEST_EVENTS.get()
        && let Ok(mut lock) = events.lock()
    {
        lock.clear();
    }
}

#[cfg(test)]
pub(crate) fn test_events_snapshot() -> Vec<TestAuditEvent> {
    TEST_EVENTS
        .get()
        .and_then(|events| events.lock().ok().map(|lock| lock.clone()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::{AuditLogger, clear_test_events, record, test_events_snapshot};

    #[test]
    fn hash_chain_links_consecutive_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");
        let logger = AuditLogger::new(path.clone(), true);

        logger.write("first", serde_json::json!({"n": 1}));
        logger.write("second", serde_json::json!({"n": 2}));

        let raw = fs::read_to_string(path).expect("read audit log");
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 2);

        let first: Value = serde_json::from_str(lines[0]).expect("first line json");
        let second: Value = serde_json::from_str(lines[1]).expect("second line json");

        let first_hash = first
            .get("hash")
            .and_then(|v| v.as_str())
            .expect("first hash")
            .to_string();
        assert!(first.get("prev_hash").map(|v| v.is_null()).unwrap_or(true));

        let second_prev = second
            .get("prev_hash")
            .and_then(|v| v.as_str())
            .expect("second prev_hash");
        assert_eq!(second_prev, first_hash);
        assert!(second.get("hash").and_then(|v| v.as_str()).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_existing_file_with_non_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit.jsonl");
        fs::write(&path, "existing\n").expect("seed existing file");
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("set permissive mode");

        let logger = AuditLogger::new(path.clone(), false);
        logger.write("event", serde_json::json!({"kind": "perm_fix"}));

        let raw = fs::read_to_string(&path).expect("read audit log");
        assert_eq!(raw, "existing\n");
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o644);
    }

    #[cfg(unix)]
    #[test]
    fn write_enforces_0600_permissions_on_new_file() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("audit-new.jsonl");
        let logger = AuditLogger::new(path.clone(), false);

        logger.write("event", serde_json::json!({"kind": "create"}));

        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn audit_hook_events_capture_metadata_only_fields() {
        clear_test_events();
        record(
            "tool_call_completed",
            serde_json::json!({
                "thread_id": "thread-1",
                "tool_name": "echo",
                "elapsed_ms": 12,
                "outcome": "ok",
                "error_kind": serde_json::Value::Null,
            }),
        );

        let events = test_events_snapshot();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "tool_call_completed");
        assert_eq!(
            events[0].details.get("thread_id").and_then(|v| v.as_str()),
            Some("thread-1")
        );
        assert!(events[0].details.get("content").is_none());
        assert!(events[0].details.get("parameters").is_none());
        assert!(events[0].details.get("output").is_none());
    }
}
