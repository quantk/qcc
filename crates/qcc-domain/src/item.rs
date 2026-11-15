use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ItemId;

/// What an item is. Bridges send full shapes; the assembler never invents content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ItemShape {
    AgentMessage {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Plan {
        text: String,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
    },
    FileChange {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    FileRead {
        path: String,
    },
    Search {
        query: String,
    },
    Tool {
        name: String,
        #[serde(default)]
        input: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
    },
    /// Plugin-defined item, `kind` is `"<plugin-id>/<name>"`.
    Extension {
        kind: String,
        #[serde(default)]
        payload: Value,
    },
}

impl ItemShape {
    /// Text of a streamed-text shape, if this is one.
    pub fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::AgentMessage { text } | Self::Reasoning { text } | Self::Plan { text } => {
                Some(text)
            }
            _ => None,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self, Self::AgentMessage { .. } | Self::Reasoning { .. } | Self::Plan { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    Pending,
    Completed,
    Failed,
    /// The user refused the approval the item was waiting for.
    Declined,
    /// Settled by the core because its turn or session went away.
    Interrupted,
}

impl ItemStatus {
    pub fn is_terminal(self) -> bool {
        self != Self::Pending
    }
}

/// A canonical timeline item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub status: ItemStatus,
    #[serde(flatten)]
    pub shape: ItemShape,
}
