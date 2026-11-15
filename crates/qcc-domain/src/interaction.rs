//! Requests a provider makes of the user mid-turn: approvals and questions.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum OptionKind {
    Allow,
    Deny,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct InteractionOption {
    pub id: String,
    pub label: String,
    pub kind: OptionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InteractionRequest {
    Approval {
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        options: Vec<InteractionOption>,
    },
    Question {
        question: String,
        #[serde(default)]
        options: Vec<InteractionOption>,
        #[serde(default)]
        allow_text: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum InteractionResolution {
    Selected {
        option_id: String,
    },
    Text {
        text: String,
    },
    /// Nobody answered: the turn settled or the session went away first.
    Cancelled,
}
