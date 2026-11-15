//! IO-free domain types of qcc.
//!
//! Two vocabularies live here:
//!
//! * [`delta`] — the narrow grammar a provider bridge speaks. A bridge parses its agent's
//!   dialect into semantic deltas and knows nothing about the timeline.
//! * [`event`] — the canonical timeline. Only the core's delta assembler constructs
//!   [`ThreadEvent`]s: it mints ids and owns ordering and lifecycle.
//!
//! This crate depends on neither an async runtime nor the plugin host.

pub mod delta;
pub mod event;
pub mod ids;
pub mod interaction;
pub mod item;
pub mod provider;
pub mod thread;
pub mod usage;

pub use delta::{Delta, ItemKey, TextChannel};
pub use event::{EventPayload, ThreadEvent};
pub use ids::{InteractionId, ItemId, ProviderId, ThreadId, TurnId};
pub use interaction::{InteractionOption, InteractionRequest, InteractionResolution, OptionKind};
pub use item::{Item, ItemShape, ItemStatus};
pub use provider::{
    ChangeKind, ChangedFile, EnvironmentChanges, EnvironmentInfo, ModelInfo, PermissionMode,
    ProviderCapabilities, ProviderInfo,
};
pub use thread::{EnvironmentState, Thread, ThreadEnvironment, ThreadStatus, TurnStatus};
pub use usage::{TokenUsage, UsageSnapshot};
