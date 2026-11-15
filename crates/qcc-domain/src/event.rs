//! The canonical timeline. Append-only, strictly ordered per thread by `seq`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{InteractionId, ItemId, ProviderId, ThreadId, TurnId};
use crate::interaction::{InteractionRequest, InteractionResolution};
use crate::item::Item;
use crate::provider::PermissionMode;
use crate::thread::{Thread, ThreadStatus, TurnStatus};
use crate::usage::UsageSnapshot;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ThreadEvent {
    pub thread_id: ThreadId,
    /// Starts at 1 and has no gaps within a thread.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub ts: u64,
    #[serde(flatten)]
    pub payload: EventPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type")]
pub enum EventPayload {
    /// Always the first event of a thread; carries the initial projection.
    #[serde(rename = "thread/created")]
    ThreadCreated { thread: Box<Thread> },

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

    #[serde(rename = "thread/permission-mode")]
    PermissionModeChanged {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<PermissionMode>,
    },

    /// The thread goes on with another agent or model. A different provider cannot
    /// resume the previous one's session: the next session starts afresh.
    #[serde(rename = "thread/agent")]
    AgentChanged {
        provider: ProviderId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },

    /// The thread was put away. Always its last state change.
    #[serde(rename = "thread/archived")]
    ThreadArchived,

    /// The environment plugin was asked (again) to make the working directory.
    #[serde(rename = "environment/preparing")]
    EnvironmentPreparing,

    /// The working directory exists; the thread works in `cwd` from now on.
    #[serde(rename = "environment/ready")]
    EnvironmentReady { cwd: String },

    #[serde(rename = "environment/failed")]
    EnvironmentFailed { message: String },

    #[serde(rename = "environment/removed")]
    EnvironmentRemoved,

    /// A user message was handed to the provider.
    #[serde(rename = "input/submitted")]
    InputSubmitted {
        text: String,
        /// Fed into the running turn instead of starting one.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        steered: bool,
        /// The queued input this delivery takes off the queue.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        queue_id: Option<String>,
    },

    /// A user message arrived while a turn was running and waits for it to end.
    #[serde(rename = "input/queued")]
    InputQueued { queue_id: String, text: String },

    /// A queued message was discarded before delivery: the thread was stopped, or the
    /// user took the message back.
    #[serde(rename = "input/dropped")]
    InputDropped { queue_id: String },

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
