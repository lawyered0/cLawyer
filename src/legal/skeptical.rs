//! Skeptical Mode policy and prompt helpers.

use std::sync::Arc;

use crate::config::{LegalConfig, LegalHardeningProfile};
use crate::db::Database;

/// User setting key for Skeptical Mode.
pub const SKEPTICAL_MODE_SETTING_KEY: &str = "skeptical_mode";

const SKEPTICAL_MODE_PROMPT_ADDENDUM: &str = r#"You are operating in Skeptical Mode. After every substantive response, append a footer with exactly this structure - no deviations:

---
**⚠ Skeptical Mode**

**Assumptions made:**
[numbered list of assumptions you made that the user did not state]

**Low-confidence points:**
[bulleted list - only include if any of these apply: (1) jurisdiction-specific law not explicitly cited, (2) factual assertions not in the user's context, (3) legal conclusions depending on unstated facts. If none apply, write "None identified."]

**Before relying on this, a lawyer should:**
[exactly 2-3 concrete, actionable verification steps specific to this response]
---

Omit the footer entirely for: acknowledgements, clarifying questions, tool status messages, or any response under 2 sentences."#;

/// Parse a persisted Skeptical Mode setting value.
///
/// Accepted values:
/// - JSON boolean (`true` / `false`)
/// - JSON string (`"true"` / `"false"`, case-insensitive)
pub fn parse_setting_value(value: &serde_json::Value) -> Option<bool> {
    match value {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::String(raw) => match raw.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve Skeptical Mode from an optional setting row and a default.
pub fn resolve_from_setting_row(
    setting_value: Option<&serde_json::Value>,
    default_enabled: bool,
) -> bool {
    setting_value
        .and_then(parse_setting_value)
        .unwrap_or(default_enabled)
}

/// Default Skeptical Mode policy from legal hardening profile.
pub fn default_enabled_for_legal(legal: &LegalConfig) -> bool {
    legal.hardening == LegalHardeningProfile::MaxLockdown
}

/// Resolve Skeptical Mode for a user from settings with fallback default.
pub async fn resolve_for_user(
    store: Option<&Arc<dyn Database>>,
    user_id: &str,
    default_enabled: bool,
) -> bool {
    let Some(store) = store else {
        return default_enabled;
    };
    match store.get_setting(user_id, SKEPTICAL_MODE_SETTING_KEY).await {
        Ok(setting) => resolve_from_setting_row(setting.as_ref(), default_enabled),
        Err(err) => {
            tracing::warn!(
                user_id,
                "Failed to read skeptical_mode setting; using default: {}",
                err
            );
            default_enabled
        }
    }
}

/// Return the canonical Skeptical Mode prompt addendum.
pub fn prompt_addendum() -> &'static str {
    SKEPTICAL_MODE_PROMPT_ADDENDUM
}

/// Append Skeptical Mode prompt instructions to a base system prompt.
pub fn append_prompt_addendum(base: String, enabled: bool) -> String {
    if !enabled {
        return base;
    }
    format!("{base}\n\n{}", prompt_addendum())
}
