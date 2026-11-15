//! What the daemon asks of a plugin, in domain terms. The WIT types stay inside this
//! crate.

use qcc_domain::{
    EnvironmentChanges, EnvironmentInfo, InteractionResolution, ModelInfo, PermissionMode,
    ProviderId, ProviderInfo, ThreadId, UiViewInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contribution {
    Provider(ProviderInfo),
    Environment(EnvironmentInfo),
    UiView(UiViewInfo),
    /// The plugin draws timeline items of this extension kind.
    UiItem(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStart {
    pub thread: ThreadId,
    pub cwd: String,
    pub model: Option<String>,
    /// Provider session handle from an earlier start of this thread.
    pub resume: Option<String>,
    pub prompt: Option<String>,
    /// Absent: the agent's own configuration decides.
    pub permission_mode: Option<PermissionMode>,
    /// Variables the agent process must see.
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequest {
    Start(SessionStart),
    Input {
        thread: ThreadId,
        text: String,
    },
    /// Input for the running turn; only for providers that declared `steer`.
    Steer {
        thread: ThreadId,
        text: String,
    },
    Interrupt {
        thread: ThreadId,
    },
    Stop {
        thread: ThreadId,
    },
    ResolveInteraction {
        thread: ThreadId,
        request_id: String,
        resolution: InteractionResolution,
    },
    ListModels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentRequest {
    Prepare {
        thread: ThreadId,
        source_dir: String,
        /// A directory set aside for the plugin's environments.
        root: String,
        base: Option<String>,
    },
    Remove {
        thread: ThreadId,
        cwd: String,
        force: bool,
    },
    Changes {
        thread: ThreadId,
        cwd: String,
    },
    Diff {
        thread: ThreadId,
        cwd: String,
    },
}

/// A request to draw. Each is answered with [`Outcome::UiTree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRequest {
    Render {
        view: String,
        thread: Option<ThreadId>,
    },
    Action {
        view: String,
        thread: Option<ThreadId>,
        action: String,
        value: Option<String>,
    },
    RenderItem {
        kind: String,
        /// The item's payload, as JSON.
        payload: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginRequest {
    Provider { provider: ProviderId, request: ProviderRequest },
    Environment { environment: String, request: EnvironmentRequest },
    Ui(UiRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    SessionStarted {
        provider_session: Option<String>,
    },
    Models(Vec<ModelInfo>),
    EnvironmentReady {
        cwd: String,
    },
    EnvironmentChanges(EnvironmentChanges),
    EnvironmentDiff {
        diff: String,
        truncated: bool,
    },
    /// A tree of UI nodes, as the JSON the plugin wrote.
    UiTree(String),
}
