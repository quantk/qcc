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
pub mod automation;
pub mod comment;
pub mod context;
pub mod delta;
pub mod event;
pub mod fork;
pub mod ids;
pub mod interaction;
pub mod item;
pub mod limits;
pub mod project;
pub mod provider;
pub mod stats;
pub mod thread;
pub mod usage;

pub use attachment::{Attachment, guess_mime, is_image_mime};
pub use automation::{
    Automation, AutomationOutcome, AutomationOverlap, AutomationRun, AutomationSpec,
    AutomationThreadMode, AutomationTrigger,
};
pub use comment::{CommentAuthor, DiffSide, LineComment};
pub use context::{ContextCategory, ContextEntry, ContextKind, ContextUsage};
pub use delta::{Compaction, Delta, ItemKey, TextChannel};
pub use event::{EventPayload, ThreadEvent, UserInput};
pub use fork::{TurnRecord, fork_blocker, turns};
pub use ids::{
    AutomationId, CommentId, InteractionId, ItemId, ProjectId, ProviderId, ThreadId, TurnId,
};
pub use interaction::{InteractionOption, InteractionRequest, InteractionResolution, OptionKind};
pub use item::{Item, ItemShape, ItemStatus};
pub use limits::{LimitStatus, LimitWindow, ProviderLimits, RateLimits};
pub use project::{ICON_EMOJI_MAX, ICON_NAME_MAX, IconColor, Project, ProjectIcon, project_of};
pub use provider::{
    AgentSkill, ChangeKind, ChangeScope, ChangedFile, CommandInfo, CommandParam, DeliverMode,
    EnvironmentChanges, EnvironmentInfo, ForkSupport, McpServer, McpServerStatus, McpState,
    McpTransport, ModelInfo, PermissionMode, ProviderCapabilities, ProviderInfo, Revision,
    SettingField, SettingKind, SkillInfo, SkillRef, UiPlacement, UiScope, UiViewInfo,
    plausible_icon,
};
pub use stats::{ToolBucket, ToolKind, ToolUse};
pub use thread::{
    ActivityKind, EnvironmentState, ForkOrigin, PendingInteraction, Role, Thread, ThreadActivity,
    ThreadEnvironment, ThreadState, ThreadStatus, TranscriptEntry, TurnStatus,
};
pub use usage::{ThreadUsage, TokenUsage, UsageBucket, UsageSnapshot};
