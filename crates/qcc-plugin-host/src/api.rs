//! What the daemon asks of a plugin, in domain terms. The WIT types stay inside this
//! crate.

use qcc_domain::{
    CommandInfo, EnvironmentChanges, EnvironmentInfo, InteractionResolution, ModelInfo,
    PermissionMode, ProviderId, ProviderInfo, SkillInfo, SkillRef, Thread, ThreadId, ThreadState,
    TranscriptEntry, UiViewInfo,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Contribution {
    Provider(ProviderInfo),
    Environment(EnvironmentInfo),
    UiView(UiViewInfo),
    /// The plugin draws timeline items of this extension kind.
    UiItem(String),
    Command(CommandInfo),
    Skill(SkillInfo),
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
    pub skills: Vec<SkillRef>,
    /// Other folders of the thread's project.
    pub extra_dirs: Vec<String>,
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
    Provider {
        provider: ProviderId,
        request: ProviderRequest,
    },
    Environment {
        environment: String,
        request: EnvironmentRequest,
    },
    Ui(UiRequest),
    /// Answered with [`Outcome::CommandResult`].
    Command {
        command: String,
        thread: Option<ThreadId>,
        args: Vec<(String, String)>,
    },
    /// A watched thread settled. Nobody waits for the answer; see
    /// [`crate::PluginHandle::post`].
    ThreadSettled(Box<SettledThread>),
}

/// A thread as plugins see it: the projection and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub thread: Thread,
    pub state: ThreadState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledThread {
    pub summary: ThreadSummary,
    pub last_message: Option<String>,
}

/// What a plugin may ask of threads through the `threads` import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadsCall {
    Spawn(SpawnThread),
    Send { thread: ThreadId, text: String },
    Get(ThreadId),
    Transcript(ThreadId),
    Watch(ThreadId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnThread {
    pub provider: ProviderId,
    pub cwd: String,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub parent: Option<ThreadId>,
    pub environment: Option<String>,
    pub base: Option<String>,
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadsAnswer {
    Done,
    Thread(Box<ThreadSummary>),
    Transcript(Vec<TranscriptEntry>),
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
    CommandResult {
        message: String,
        thread: Option<ThreadId>,
    },
}
