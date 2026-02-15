//! Accessibility tree parsing and element reference generation.
//!
//! Converts Chrome's CDP accessibility tree into a compact, LLM-friendly
//! representation with stable element references (`@e1`, `@e2`, ...).
//!
//! The key insight: sending the full accessibility tree every turn is wasteful.
//! Instead, we assign short IDs to interactive elements and let the LLM
//! reference them by ID for clicks/typing. This is ~93% cheaper in tokens
//! compared to re-sending the full tree each time.
//!
//! ```text
//! Page: https://example.com/login
//! @e1: textbox "Email" [focused]
//! @e2: textbox "Password" [type=password]
//! @e3: button "Sign In"
//! @e4: link "Forgot password?"
//! ```

use std::collections::HashMap;
use std::fmt;

use chromiumoxide::cdp::browser_protocol::accessibility::{AxNode, AxPropertyName};
use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;

/// A resolved element reference that maps `@eN` back to a DOM target.
#[derive(Debug, Clone)]
pub struct ElementRef {
    /// The display label shown to the LLM (e.g., `textbox "Email"`).
    #[allow(dead_code)]
    pub label: String,
    /// CDP backend node ID for targeting this element.
    pub backend_node_id: BackendNodeId,
    /// CSS selector hint (best-effort, may not be unique).
    #[allow(dead_code)]
    pub selector_hint: Option<String>,
}

/// Stores the current set of element references for a page snapshot.
#[derive(Debug, Clone, Default)]
pub struct ElementRefMap {
    refs: HashMap<String, ElementRef>,
    counter: usize,
}

impl ElementRefMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a reference like `@e1` or just `e1`.
    pub fn get(&self, ref_id: &str) -> Option<&ElementRef> {
        let normalized = ref_id.strip_prefix('@').unwrap_or(ref_id);
        self.refs.get(normalized)
    }

    /// Number of tracked elements.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.refs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    /// Reset all refs. Called before each new `read_page` and when switching tabs.
    pub fn reset(&mut self) {
        self.refs.clear();
        self.counter = 0;
    }

    /// Allocate the next reference ID and store the element.
    fn insert(&mut self, elem: ElementRef) -> String {
        self.counter += 1;
        let id = format!("e{}", self.counter);
        self.refs.insert(id.clone(), elem);
        id
    }
}

/// Which elements to include when building the tree representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementFilter {
    /// Only interactive elements (buttons, links, inputs, selects, textareas).
    Interactive,
    /// All elements with meaningful content.
    All,
}

impl ElementFilter {
    pub fn from_str_opt(s: Option<&str>) -> Self {
        match s {
            Some("all") => Self::All,
            _ => Self::Interactive,
        }
    }
}

/// Roles that are considered "interactive" for filtering purposes.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "searchbox",
    "combobox",
    "listbox",
    "option",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "radio",
    "checkbox",
    "switch",
    "slider",
    "spinbutton",
    "tab",
    "treeitem",
];

/// Roles to skip entirely (structural noise).
const SKIP_ROLES: &[&str] = &[
    "none",
    "presentation",
    "generic",
    "InlineTextBox",
    "LineBreak",
];

/// Build a compact page representation from the CDP accessibility tree.
///
/// Returns the text representation and populates `ref_map` with element
/// references the LLM can use for subsequent actions.
pub fn build_page_repr(
    url: &str,
    title: &str,
    nodes: &[AxNode],
    filter: ElementFilter,
    ref_map: &mut ElementRefMap,
) -> String {
    ref_map.reset();

    let mut lines = Vec::new();

    // Header
    lines.push(format!("Page: {}", url));
    if !title.is_empty() {
        lines.push(format!("Title: {}", title));
    }
    lines.push(String::new());

    // Walk nodes, collecting elements that pass the filter.
    for node in nodes {
        let role = node_role(node);

        if SKIP_ROLES.contains(&role.as_str()) {
            continue;
        }

        // For "interactive" filter, only include interactive roles.
        if filter == ElementFilter::Interactive && !INTERACTIVE_ROLES.contains(&role.as_str()) {
            continue;
        }

        // Skip nodes without a name (usually decorative).
        let name = node_name(node);
        if name.is_empty() && filter == ElementFilter::Interactive {
            continue;
        }

        let backend_id = match node.backend_dom_node_id {
            Some(id) => id,
            None => continue,
        };

        // Build display label
        let mut label = NodeLabel {
            role: role.clone(),
            name: truncate_name(&name, 80),
            properties: Vec::new(),
        };

        // Add useful properties
        if node_has_property(node, "focused") {
            label.properties.push("focused".to_string());
        }
        if node_has_property(node, "checked") {
            label.properties.push("checked".to_string());
        }
        if node_has_property(node, "disabled") {
            label.properties.push("disabled".to_string());
        }
        if node_has_property(node, "expanded") {
            label.properties.push("expanded".to_string());
        }
        if node_has_property(node, "required") {
            label.properties.push("required".to_string());
        }
        if let Some(val) = node_value(node) {
            if !val.is_empty() && val != name {
                label
                    .properties
                    .push(format!("value=\"{}\"", truncate_name(&val, 40)));
            }
        }

        let display = label.to_string();

        let elem_ref = ElementRef {
            label: display.clone(),
            backend_node_id: backend_id,
            selector_hint: guess_selector(node),
        };

        let ref_id = ref_map.insert(elem_ref);
        lines.push(format!("@{}: {}", ref_id, display));
    }

    if ref_map.is_empty() {
        lines.push("(no interactive elements found)".to_string());
    }

    lines.join("\n")
}

/// Extract the role string from an AX node.
fn node_role(node: &AxNode) -> String {
    node.role
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Extract the name (accessible label) from an AX node.
fn node_name(node: &AxNode) -> String {
    node.name
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Extract the value from an AX node (for inputs, etc.).
fn node_value(node: &AxNode) -> Option<String> {
    node.value
        .as_ref()
        .and_then(|v| v.value.as_ref())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Map a property name string to the corresponding `AxPropertyName` variant.
fn property_by_name(name: &str) -> Option<AxPropertyName> {
    match name {
        "focused" => Some(AxPropertyName::Focused),
        "checked" => Some(AxPropertyName::Checked),
        "disabled" => Some(AxPropertyName::Disabled),
        "expanded" => Some(AxPropertyName::Expanded),
        "required" => Some(AxPropertyName::Required),
        "selected" => Some(AxPropertyName::Selected),
        "pressed" => Some(AxPropertyName::Pressed),
        "readonly" => Some(AxPropertyName::Readonly),
        "hidden" => Some(AxPropertyName::Hidden),
        "modal" => Some(AxPropertyName::Modal),
        _ => None,
    }
}

/// Check if a node has a boolean property set to true.
fn node_has_property(node: &AxNode, prop_name: &str) -> bool {
    let Some(props) = &node.properties else {
        return false;
    };
    let Some(target) = property_by_name(prop_name) else {
        return false;
    };
    props.iter().any(|p| {
        p.name == target
            && p.value
                .value
                .as_ref()
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
    })
}

/// Best-effort CSS selector guess from node attributes.
fn guess_selector(node: &AxNode) -> Option<String> {
    // We don't have DOM attributes directly from the AX tree,
    // so we can only offer role-based hints. The actual targeting
    // uses backend_node_id which is precise.
    let role = node_role(node);
    let name = node_name(node);

    if name.is_empty() {
        return None;
    }

    // Build an ARIA selector hint (not used for actual targeting,
    // just a human-readable hint in debug output).
    Some(format!(
        "[role=\"{}\"][name=\"{}\"]",
        role,
        truncate_name(&name, 30)
    ))
}

/// Truncate a display name to max chars, adding ellipsis if needed.
fn truncate_name(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars().take(max.saturating_sub(3)).collect::<String>()
        )
    }
}

/// Helper for formatting a node's display label.
struct NodeLabel {
    role: String,
    name: String,
    properties: Vec<String>,
}

impl fmt::Display for NodeLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.role)?;
        if !self.name.is_empty() {
            write!(f, " \"{}\"", self.name)?;
        }
        if !self.properties.is_empty() {
            write!(f, " [{}]", self.properties.join(", "))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::tools::builtin::browser::accessibility::{
        ElementFilter, ElementRefMap, build_page_repr, truncate_name,
    };
    use chromiumoxide::cdp::browser_protocol::accessibility::{
        AxNode, AxNodeId, AxValue, AxValueType,
    };
    use chromiumoxide::cdp::browser_protocol::dom::BackendNodeId;

    fn make_ax_value(s: &str) -> AxValue {
        let mut v = AxValue::new(AxValueType::String);
        v.value = Some(serde_json::Value::String(s.to_string()));
        v
    }

    fn make_ax_node(role: &str, name: &str, backend_id: i64) -> AxNode {
        let mut node = AxNode::new(AxNodeId::from(format!("node_{}", backend_id)), false);
        node.role = Some(make_ax_value(role));
        node.name = Some(make_ax_value(name));
        node.backend_dom_node_id = Some(BackendNodeId::new(backend_id));
        node
    }

    #[test]
    fn test_build_page_repr_interactive_filter() {
        let nodes = vec![
            make_ax_node("button", "Submit", 1),
            make_ax_node("link", "Home", 2),
            make_ax_node("textbox", "Email", 3),
            make_ax_node("heading", "Welcome", 4), // not interactive
            make_ax_node("generic", "", 5),        // skip role
        ];

        let mut ref_map = ElementRefMap::new();
        let repr = build_page_repr(
            "https://example.com",
            "Test Page",
            &nodes,
            ElementFilter::Interactive,
            &mut ref_map,
        );

        assert!(repr.contains("@e1: button \"Submit\""));
        assert!(repr.contains("@e2: link \"Home\""));
        assert!(repr.contains("@e3: textbox \"Email\""));
        assert!(!repr.contains("heading"));
        assert!(!repr.contains("generic"));
        assert_eq!(ref_map.len(), 3);
    }

    #[test]
    fn test_build_page_repr_all_filter() {
        let nodes = vec![
            make_ax_node("button", "Submit", 1),
            make_ax_node("heading", "Welcome", 2),
        ];

        let mut ref_map = ElementRefMap::new();
        let repr = build_page_repr(
            "https://example.com",
            "",
            &nodes,
            ElementFilter::All,
            &mut ref_map,
        );

        assert!(repr.contains("button"));
        assert!(repr.contains("heading"));
        assert_eq!(ref_map.len(), 2);
    }

    #[test]
    fn test_element_ref_lookup() {
        let mut ref_map = ElementRefMap::new();
        let nodes = vec![make_ax_node("button", "Click me", 1)];
        build_page_repr(
            "https://x.com",
            "",
            &nodes,
            ElementFilter::Interactive,
            &mut ref_map,
        );

        assert!(ref_map.get("e1").is_some());
        assert!(ref_map.get("@e1").is_some()); // with @ prefix
        assert!(ref_map.get("e99").is_none());
    }

    #[test]
    fn test_empty_page() {
        let mut ref_map = ElementRefMap::new();
        let repr = build_page_repr(
            "https://empty.com",
            "",
            &[],
            ElementFilter::Interactive,
            &mut ref_map,
        );

        assert!(repr.contains("no interactive elements"));
        assert!(ref_map.is_empty());
    }

    #[test]
    fn test_truncate_name() {
        assert_eq!(truncate_name("short", 10), "short");
        assert_eq!(truncate_name("this is a very long name", 10), "this is...");
    }
}
