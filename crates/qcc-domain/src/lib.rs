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

pub mod attachment;
pub mod comment;
pub mod context;
pub mod delta;
pub mod event;
pub mod ids;
pub mod interaction;
pub mod item;
pub mod project;
pub mod provider;
pub mod stats;
pub mod thread;
pub mod usage;

pub use attachment::{Attachment, guess_mime, is_image_mime};
pub use comment::{DiffSide, LineComment};
pub use context::{ContextCategory, ContextEntry, ContextKind, ContextUsage};
pub use delta::{Compaction, Delta, ItemKey, TextChannel};
pub use event::{EventPayload, ThreadEvent, UserInput};
pub use ids::{CommentId, InteractionId, ItemId, ProjectId, ProviderId, ThreadId, TurnId};
pub use interaction::{InteractionOption, InteractionRequest, InteractionResolution, OptionKind};
pub use item::{Item, ItemShape, ItemStatus};
pub use project::{Project, project_of};
pub use provider::{
    AgentSkill, ChangeKind, ChangedFile, CommandInfo, CommandParam, DeliverMode,
    EnvironmentChanges, EnvironmentInfo, McpServer, McpServerStatus, McpState, McpTransport,
    ModelInfo, PermissionMode, ProviderCapabilities, ProviderInfo, SettingField, SettingKind,
    SkillInfo, SkillRef, UiPlacement, UiScope, UiViewInfo,
};
pub use stats::{ToolBucket, ToolKind, ToolUse};
pub use thread::{
    ActivityKind, EnvironmentState, PendingInteraction, Role, Thread, ThreadActivity,
    ThreadEnvironment, ThreadState, ThreadStatus, TranscriptEntry, TurnStatus,
};
pub use usage::{ThreadUsage, TokenUsage, UsageBucket, UsageSnapshot};
