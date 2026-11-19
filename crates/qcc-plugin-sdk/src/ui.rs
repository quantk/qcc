//! Builders for the UI nodes a view is made of.
//!
//! ```ignore
//! use qcc_plugin_sdk::ui::*;
//!
//! let tree = stack([
//!     heading("Deploys"),
//!     key_values([("Last", "2 hours ago"), ("Status", "ok")]),
//!     button("Deploy now", "deploy").primary(),
//! ]);
//! Ok(tree.to_json())
//! ```
//!
//! The set is fixed on purpose: the app decides how each node looks, so every
//! plugin's view reads like the rest of the app and none can run code in it.

use serde_json::{Map, Value, json};

/// One node of a view.
#[derive(Debug, Clone, PartialEq)]
pub struct Node(Map<String, Value>);

/// How a node is coloured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Muted,
    Success,
    Warning,
    Danger,
}

impl Tone {
    fn as_str(self) -> &'static str {
        match self {
            Self::Muted => "muted",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Danger => "danger",
        }
    }
}

impl Node {
    fn new(kind: &str, fields: Value) -> Self {
        let mut map = Map::new();
        map.insert("type".into(), kind.into());
        if let Value::Object(fields) = fields {
            map.extend(fields);
        }
        Self(map)
    }

    fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.0.insert(key.into(), value.into());
        self
    }

    /// Colour a `text` or a `badge`.
    pub fn tone(self, tone: Tone) -> Self {
        self.with("tone", tone.as_str())
    }

    /// Make a `button` the main action of its view.
    pub fn primary(self) -> Self {
        self.with("style", "primary")
    }

    /// Mark a `button` as destructive.
    pub fn danger(self) -> Self {
        self.with("style", "danger")
    }

    /// What a `button` sends along with its action; the text an `input` starts with
    /// (and keeps once submitted).
    pub fn value(self, value: impl Into<String>) -> Self {
        self.with("value", value.into())
    }

    pub fn to_value(&self) -> Value {
        Value::Object(self.0.clone())
    }

    /// The tree as the JSON a `Ui` method answers with.
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }
}

fn children(nodes: impl IntoIterator<Item = Node>) -> Value {
    Value::Array(nodes.into_iter().map(|node| Value::Object(node.0)).collect())
}

/// Nodes one under another.
pub fn stack(nodes: impl IntoIterator<Item = Node>) -> Node {
    Node::new("stack", json!({ "children": children(nodes) }))
}

/// Nodes side by side, wrapping when they do not fit.
pub fn row(nodes: impl IntoIterator<Item = Node>) -> Node {
    Node::new("row", json!({ "children": children(nodes) }))
}

pub fn heading(text: impl Into<String>) -> Node {
    Node::new("heading", json!({ "text": text.into() }))
}

pub fn text(text: impl Into<String>) -> Node {
    Node::new("text", json!({ "text": text.into() }))
}

pub fn markdown(text: impl Into<String>) -> Node {
    Node::new("markdown", json!({ "text": text.into() }))
}

/// Preformatted text.
pub fn code(text: impl Into<String>) -> Node {
    Node::new("code", json!({ "text": text.into() }))
}

pub fn badge(text: impl Into<String>) -> Node {
    Node::new("badge", json!({ "text": text.into() }))
}

pub fn divider() -> Node {
    Node::new("divider", json!({}))
}

pub fn key_values<K: Into<String>, V: Into<String>>(
    items: impl IntoIterator<Item = (K, V)>,
) -> Node {
    let items: Vec<Value> = items
        .into_iter()
        .map(|(key, value)| json!({ "key": key.into(), "value": value.into() }))
        .collect();
    Node::new("keyValues", json!({ "items": items }))
}

pub fn table<C: Into<String>, R: IntoIterator<Item = String>>(
    columns: impl IntoIterator<Item = C>,
    rows: impl IntoIterator<Item = R>,
) -> Node {
    let columns: Vec<String> = columns.into_iter().map(Into::into).collect();
    let rows: Vec<Vec<String>> = rows.into_iter().map(|row| row.into_iter().collect()).collect();
    Node::new("table", json!({ "columns": columns, "rows": rows }))
}

/// Sends `action` to the plugin when pressed.
pub fn button(label: impl Into<String>, action: impl Into<String>) -> Node {
    Node::new("button", json!({ "label": label.into(), "action": action.into() }))
}

/// A line of text the user submits: `action` arrives with what was typed.
pub fn input(
    placeholder: impl Into<String>,
    submit: impl Into<String>,
    action: impl Into<String>,
) -> Node {
    Node::new(
        "input",
        json!({ "placeholder": placeholder.into(), "submit": submit.into(), "action": action.into() }),
    )
}

/// A choice among `options`, each `(value, label)`: `action` arrives with the
/// value the user picks.
pub fn select<V: Into<String>, L: Into<String>>(
    label: impl Into<String>,
    options: impl IntoIterator<Item = (V, L)>,
    selected: impl Into<String>,
    action: impl Into<String>,
) -> Node {
    let options: Vec<Value> = options
        .into_iter()
        .map(|(value, label)| json!({ "value": value.into(), "label": label.into() }))
        .collect();
    Node::new(
        "select",
        json!({
            "label": label.into(),
            "options": options,
            "value": selected.into(),
            "action": action.into(),
        }),
    )
}

/// Text that opens a place in the app when clicked: a thread, or with `path` (and
/// `line`) that file of the thread's changes. The app decides how it opens it; a
/// plugin cannot point anywhere else.
pub fn link(
    label: impl Into<String>,
    thread: impl Into<String>,
    path: Option<&str>,
    line: Option<u32>,
) -> Node {
    let mut node = Node::new("link", json!({ "label": label.into(), "thread": thread.into() }));
    if let Some(path) = path {
        node = node.with("path", path);
    }
    if let Some(line) = line {
        node = node.with("line", line);
    }
    node
}

/// An on/off switch: `action` arrives with `"true"` or `"false"`, what it is now.
pub fn switch(label: impl Into<String>, on: bool, action: impl Into<String>) -> Node {
    Node::new("switch", json!({ "label": label.into(), "checked": on, "action": action.into() }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_serializes_to_tagged_objects() {
        let tree = stack([
            heading("Title"),
            row([badge("ok").tone(Tone::Success), button("Go", "go").primary().value("1")]),
            key_values([("a", "b")]),
            table(["x"], [vec!["1".to_owned()]]),
            input("note", "Save", "save"),
            select("Agent", [("codex", "Codex")], "codex", "agent"),
            switch("On", true, "on"),
            link("a.rs:3", "thr_1", Some("a.rs"), Some(3)),
        ]);
        assert_eq!(
            tree.to_value(),
            json!({
                "type": "stack",
                "children": [
                    { "type": "heading", "text": "Title" },
                    { "type": "row", "children": [
                        { "type": "badge", "text": "ok", "tone": "success" },
                        { "type": "button", "label": "Go", "action": "go", "style": "primary", "value": "1" },
                    ]},
                    { "type": "keyValues", "items": [{ "key": "a", "value": "b" }] },
                    { "type": "table", "columns": ["x"], "rows": [["1"]] },
                    { "type": "input", "placeholder": "note", "submit": "Save", "action": "save" },
                    { "type": "select", "label": "Agent", "value": "codex", "action": "agent",
                      "options": [{ "value": "codex", "label": "Codex" }] },
                    { "type": "switch", "label": "On", "checked": true, "action": "on" },
                    { "type": "link", "label": "a.rs:3", "thread": "thr_1", "path": "a.rs", "line": 3 },
                ],
            })
        );
    }
}
