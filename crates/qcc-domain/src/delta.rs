//! The delta grammar: what a provider bridge is allowed to say.
//!
//! A delta is a parsed *semantic* unit — never a raw provider event and never a finished
//! [`crate::ThreadEvent`]. Deltas carry provider-native join keys ([`ItemKey`]); the delta
//! assembler maps them to the ids it mints.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::interaction::InteractionRequest;
use crate::item::{ItemShape, ItemStatus};
use crate::provider::McpServerStatus;
use crate::thread::TurnStatus;
use crate::usage::UsageSnapshot;

/// Provider-side identity of an item within a session.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "camelCase")]
pub enum ItemKey {
    /// The provider names its items (tool-call id, message id).
    Provider(String),
    /// An anonymous stream; the bridge picks the name (`assistant`, `thinking-2`).
    /// The key is released when the stream closes or a non-text item opens.
    Stream(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TextChannel {
    AgentMessage,
    Reasoning,
    Plan,
}

impl TextChannel {
    pub fn shape(self, text: String) -> ItemShape {
        match self {
            Self::AgentMessage => ItemShape::AgentMessage { text },
            Self::Reasoning => ItemShape::Reasoning { text },
            Self::Plan => ItemShape::Plan { text },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind")]
pub enum Delta {
    /// The provider took a prompt (or started work on its own). Only this delta and a
    /// claiming [`Delta::TurnBoundary`] ever open a turn.
    #[serde(rename = "turn.open")]
    TurnOpen,

    /// The turn reached a terminal state.
    #[serde(rename = "turn.boundary")]
    TurnBoundary {
        status: TurnStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        /// With no turn open, own one only if submitted input is still waiting for a
        /// turn. A provider-terminal fallback signal on an idle thread settles nothing.
        #[serde(default)]
        claim_if_idle: bool,
    },

    #[serde(rename = "item.open")]
    ItemOpen { key: ItemKey, shape: ItemShape },

    /// Always carries the full terminal shape.
    #[serde(rename = "item.close")]
    ItemClose { key: ItemKey, shape: ItemShape, status: ItemStatus },

    /// Streamed text. Opens the item on first sight of the key.
    #[serde(rename = "item.text-delta")]
    ItemTextDelta { key: ItemKey, channel: TextChannel, text: String },

    /// Settles a text stream with the provider-final `text` or, absent that, with what
    /// was streamed.
    #[serde(rename = "item.text-close")]
    ItemTextClose {
        key: ItemKey,
        channel: TextChannel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },

    #[serde(rename = "usage")]
    Usage { usage: UsageSnapshot },

    /// The provider needs the user: an approval or a question. `request_id` is the
    /// provider-side handle the resolution is sent back with.
    #[serde(rename = "interaction.request")]
    InteractionRequest {
        request_id: String,
        /// The item the request is about, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item: Option<ItemKey>,
        request: InteractionRequest,
    },

    /// The provider's durable handle for this session, for a bridge that learns it
    /// only after `start` returned (the agent answers its handshake later).
    #[serde(rename = "session.identity")]
    SessionIdentity { provider_session: String },

    /// A fresh provider session was constructed: provider-native ids start over.
    #[serde(rename = "session.reset")]
    SessionReset,

    /// The provider session is gone. Settles whatever is still open.
    #[serde(rename = "session.ended")]
    SessionEnded {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    /// How one MCP server of the session is doing now. A fresh session starts with
    /// none known.
    #[serde(rename = "mcp.status")]
    McpStatus { server: McpServerStatus },

    #[serde(rename = "provider.error")]
    ProviderError { message: String },

    #[serde(rename = "provider.warning")]
    ProviderWarning { message: String },

    /// Provider traffic the bridge recognised as unknown. Kept for diagnostics.
    #[serde(rename = "unhandled")]
    Unhandled { payload: Value },
}
