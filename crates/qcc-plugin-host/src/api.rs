//! What the daemon asks of a plugin, in domain terms. The WIT types stay inside this
//! crate.

use qcc_domain::{
    AgentSkill, ChangeScope, CommandInfo, CommentAuthor, DeliverMode, DiffSide, EnvironmentChanges,
    EnvironmentInfo, InteractionResolution, LineComment, McpServer, ModelInfo, PermissionMode,
    ProviderId, ProviderInfo, RateLimits, SettingField, SkillInfo, SkillRef, Thread, ThreadId,
    ThreadState, TranscriptEntry, TurnStatus, UiViewInfo, UserInput,
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
    Setting(SettingField),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStart {
    pub thread: ThreadId,
    pub cwd: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Provider session handle from an earlier start of this thread.
    pub resume: Option<String>,
    pub prompt: Option<UserInput>,
    /// Absent: the agent's own configuration decides.
    pub permission_mode: Option<PermissionMode>,
    /// Variables the agent process must see.
    pub env: Vec<(String, String)>,
    pub skills: Vec<SkillRef>,
    /// Other folders of the thread's project.
    pub extra_dirs: Vec<String>,
    /// MCP servers to add to the agent's own: those of qcc the thread wants.
    pub mcp_servers: Vec<McpServer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequest {
    Start(Box<SessionStart>),
    Input {
        thread: ThreadId,
        input: UserInput,
    },
    /// Input for the running turn; only for providers that declared `steer`.
    Steer {
        thread: ThreadId,
        input: UserInput,
    },
    Interrupt {
        thread: ThreadId,
    },
    Stop {
        thread: ThreadId,
    },
    /// The qcc MCP servers the running session should have now, replacing those it
    /// got. Only for providers that declared `mcp_live`.
    SetMcpServers {
        thread: ThreadId,
        servers: Vec<McpServer>,
    },
    ResolveInteraction {
        thread: ThreadId,
        request_id: String,
        resolution: InteractionResolution,
    },
    ListModels,
    ListSkills,
    /// The account's usage limits now. Only for providers that declared `rate_limits`.
    ReadRateLimits,
    /// Only for providers that declared `compact`, while no turn runs.
    Compact {
        thread: ThreadId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentRequest {
    Prepare {
        thread: ThreadId,
        source_dir: String,
        /// A directory set aside for the plugin's environments.
        root: String,
        base: Option<String>,
        /// Take the uncommitted work of `source_dir` along into the new directory.
        carry_changes: bool,
        /// Where the user asked the directory to be, instead of inside `root`.
        dir: Option<String>,
        /// What the user asked the branch to be called.
        branch: Option<String>,
        /// `branch` may exist already.
        reuse_branch: bool,
    },
    Remove {
        thread: ThreadId,
        cwd: String,
        force: bool,
    },
    Changes {
        thread: ThreadId,
        cwd: String,
        /// `cwd` is the thread's own directory, not one of the plugin's environments.
        inspect: bool,
        scope: ChangeScope,
    },
    Diff {
        thread: ThreadId,
        cwd: String,
        /// `cwd` is the thread's own directory, not one of the plugin's environments.
        inspect: bool,
        /// Only these files; every file when empty.
        paths: Vec<String>,
        scope: ChangeScope,
        /// Whole files as context, not a few lines around each change.
        full: bool,
    },
    /// Answered with [`Outcome::EnvironmentDelivered`].
    Deliver {
        thread: ThreadId,
        cwd: String,
        mode: DeliverMode,
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
    /// The plugin's settings, by id, defaults filled in. Posted like
    /// [`PluginRequest::ThreadSettled`].
    Settings(Vec<(String, String)>),
    /// A turn of some thread ended; for plugins with the `turn_events` permission.
    /// Posted like [`PluginRequest::ThreadSettled`].
    TurnEnded {
        summary: Box<ThreadSummary>,
        status: TurnStatus,
    },
    /// The answer to a [`ThreadsCall::Diff`]. Posted like
    /// [`PluginRequest::ThreadSettled`].
    ThreadDiff {
        request: u64,
        thread: ThreadId,
        answer: Result<(String, bool), String>,
    },
    /// The answer to a [`ThreadsCall::Changes`]. Posted like
    /// [`PluginRequest::ThreadSettled`].
    ThreadChanges {
        request: u64,
        thread: ThreadId,
        answer: Result<EnvironmentChanges, String>,
    },
    /// The answer to a [`ThreadsCall::Models`]. Posted like
    /// [`PluginRequest::ThreadSettled`].
    ProviderModels {
        request: u64,
        provider: ProviderId,
        answer: Result<Vec<ModelInfo>, String>,
    },
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
    Send {
        thread: ThreadId,
        text: String,
    },
    Get(ThreadId),
    Transcript(ThreadId),
    Watch(ThreadId),
    /// Ask for the thread's diff; the answer is posted as
    /// [`PluginRequest::ThreadDiff`] with `request`, which the host picks.
    Diff {
        thread: ThreadId,
        request: u64,
    },
    /// Ask for the files the thread changed; answered like [`ThreadsCall::Diff`],
    /// with [`PluginRequest::ThreadChanges`].
    Changes {
        thread: ThreadId,
        request: u64,
    },
    /// Turn one of the plugin's own skills on or off in a thread.
    SetSkill {
        thread: ThreadId,
        skill: String,
        enabled: bool,
    },
    Providers,
    /// Ask for a provider's models; the answer is posted as
    /// [`PluginRequest::ProviderModels`] with `request`, which the host picks.
    Models {
        provider: ProviderId,
        refresh: bool,
        request: u64,
    },
    /// The `views` import, which needs no permission: the plugin's own view shows
    /// something else now.
    RefreshView {
        view: String,
        thread: Option<ThreadId>,
    },
    /// The `comments` import, checked against the `comments` permission instead.
    AddComment(NewComment),
    /// The `comments` import: the thread's comments, resolved ones if asked for.
    ListComments {
        thread: ThreadId,
        include_resolved: bool,
    },
    /// The `files` import, checked against the `files` permission: a file's text.
    ReadFile {
        root: FileRoot,
        path: String,
    },
    /// The `files` import: the names in a folder.
    ListFiles {
        root: FileRoot,
        dir: String,
    },
}

/// Where a plugin's `files` call reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileRoot {
    /// The thread's working directory.
    Thread(ThreadId),
    /// The plugin's own data folder.
    Own,
}

/// A comment a plugin leaves. Its author's `plugin` is the caller, whatever the
/// plugin says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub thread: ThreadId,
    pub path: String,
    pub line: u32,
    pub side: DiffSide,
    pub quote: Option<String>,
    pub text: String,
    pub author: CommentAuthor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnThread {
    pub provider: ProviderId,
    /// Absent: a directory of the plugin's own.
    pub cwd: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub parent: Option<ThreadId>,
    pub environment: Option<String>,
    pub base: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub skills_enabled: Vec<String>,
    pub skills_disabled: Vec<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadsAnswer {
    Done,
    Thread(Box<ThreadSummary>),
    Transcript(Vec<TranscriptEntry>),
    Providers(Vec<ProviderInfo>),
    Comment(Box<LineComment>),
    Comments(Vec<LineComment>),
    /// A file's text; `None` when there is no such file.
    File(Option<String>),
    Names(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    SessionStarted {
        provider_session: Option<String>,
    },
    Models(Vec<ModelInfo>),
    Skills(Vec<AgentSkill>),
    RateLimits(RateLimits),
    EnvironmentReady {
        cwd: String,
    },
    EnvironmentChanges(EnvironmentChanges),
    EnvironmentDiff {
        diff: String,
        truncated: bool,
    },
    /// What bringing the work back did, in a sentence.
    EnvironmentDelivered(String),
    /// A tree of UI nodes, as the JSON the plugin wrote.
    UiTree(String),
    CommandResult {
        message: String,
        thread: Option<ThreadId>,
    },
}
