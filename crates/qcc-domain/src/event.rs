//! The canonical timeline. Append-only, strictly ordered per thread by `seq`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{InteractionId, ItemId, ThreadId, TurnId};
use crate::interaction::{InteractionRequest, InteractionResolution};
use crate::item::Item;
use crate::thread::{Thread, ThreadStatus, TurnStatus};
use crate::usage::UsageSnapshot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadEvent {
    pub thread_id: ThreadId,
    /// Starts at 1 and has no gaps within a thread.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    #[serde(flatten)]
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    /// Always the first event of a thread; carries the initial projection.
    #[serde(rename = "thread/created")]
    ThreadCreated { thread: Thread },

    #[serde(rename = "thread/status")]
    ThreadStatusChanged { status: ThreadStatus },

    #[serde(rename = "session/started")]
    SessionStarted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_session: Option<String>,
        resumed: bool,
    },

    /// The provider named its session after it had started.
    #[serde(rename = "session/identified")]
    SessionIdentified { provider_session: String },

    #[serde(rename = "session/ended")]
    SessionEnded {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    #[serde(rename = "input/submitted")]
    InputSubmitted { text: String },

    #[serde(rename = "turn/started")]
    TurnStarted { turn_id: TurnId },

    #[serde(rename = "turn/completed")]
    TurnCompleted {
        turn_id: TurnId,
        status: TurnStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    /// Every item's first event.
    #[serde(rename = "item/started")]
    ItemStarted { turn_id: TurnId, item: Item },

    #[serde(rename = "item/text/delta")]
    ItemTextDelta { item_id: ItemId, text: String },

    /// Carries the full terminal item.
    #[serde(rename = "item/completed")]
    ItemCompleted { turn_id: TurnId, item: Item },

    #[serde(rename = "usage/updated")]
    UsageUpdated { usage: UsageSnapshot },

    #[serde(rename = "interaction/requested")]
    InteractionRequested {
        interaction_id: InteractionId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        turn_id: Option<TurnId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        item_id: Option<ItemId>,
        request: InteractionRequest,
    },

    #[serde(rename = "interaction/resolved")]
    InteractionResolved { interaction_id: InteractionId, resolution: InteractionResolution },

    #[serde(rename = "provider/error")]
    ProviderError { message: String },

    #[serde(rename = "provider/warning")]
    ProviderWarning { message: String },

    #[serde(rename = "provider/unhandled")]
    ProviderUnhandled { payload: Value },
}
