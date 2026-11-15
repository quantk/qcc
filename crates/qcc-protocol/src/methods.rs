//! Methods and notifications of the daemon API.

use std::collections::BTreeMap;

use qcc_domain::{
    CommandInfo, DeliverMode, EnvironmentChanges, EnvironmentInfo, InteractionId,
    InteractionResolution, McpTransport, ModelInfo, PermissionMode, Project, ProjectId, ProviderId,
    ProviderInfo, Thread, ThreadEvent, ThreadId, ThreadState, UiViewInfo,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::Method;

macro_rules! method {
    ($(#[$doc:meta])* $ty:ident, $name:literal, $params:ty => $result:ty) => {
        $(#[$doc])*
        pub struct $ty;

        impl Method for $ty {
            const NAME: &'static str = $name;
            type Params = $params;
            type Result = $result;
        }
    };
}

/// Parameters or result of a method that carries none.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Empty {}

// ----- daemon -----------------------------------------------------------------------

method!(
    /// Liveness and identity of the daemon.
    DaemonStatus, "daemon.status", Empty => DaemonStatusResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DaemonStatusResult {
    pub version: String,
    pub protocol: u32,
    pub pid: u32,
    pub data_dir: String,
    pub socket: String,
}

// ----- threads ----------------------------------------------------------------------

method!(
    /// Create a thread and start its provider session. Returns as soon as the thread
    /// exists; progress arrives as events.
    ThreadSpawn, "thread.spawn", ThreadSpawnParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSpawnParams {
    pub provider: ProviderId,
    /// Working directory of the thread, or what its environment is made from.
    pub cwd: String,
    /// Id of a contributed environment that makes the thread a working directory
    /// of its own from `cwd` (`git-worktree`). Absent: the thread works in `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// What the environment starts from (a git ref). Absent: its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How hard the model thinks: a level the provider lists for the model.
    /// Absent: the parent's (with the same agent), the user's default for the
    /// agent, or else the agent's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The thread that spawned this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    /// The project to group it under. Absent: the parent's project, or else the
    /// project one of whose folders holds `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    /// How much the agent may do unasked. Absent: the agent's own configuration
    /// decides. Must be a mode the provider lists in its capabilities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadResult {
    pub thread: Thread,
}

method!(
    /// Send a user message. With no turn running it starts one, starting or resuming
    /// the provider session if none is attached. While a turn runs it is steered into
    /// that turn or queued behind it, as `delivery` asks.
    ThreadSend, "thread.send", ThreadSendParams => ThreadSendResult
);

/// What to do with a message that arrives while a turn is running.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SendMode {
    /// Deliver it as its own turn once the running one ends.
    #[default]
    Queue,
    /// Feed it into the running turn. Falls back to queueing when the provider
    /// cannot steer.
    Steer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSendParams {
    pub thread_id: ThreadId,
    pub text: String,
    #[serde(default)]
    pub delivery: SendMode,
    /// What the message is about. qcc tells the agent in a short note along with
    /// it; the note is kept in the timeline with the message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<MessageContext>,
}

/// Something a user's message is about, which the agent is told along with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageContext {
    /// The user is looking at this thread.
    Thread { thread_id: ThreadId },
}

/// What became of a sent message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    /// It started a turn.
    Started,
    /// It went into the running turn.
    Steered,
    /// It waits for the running turn to end.
    Queued,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSendResult {
    pub delivery: Delivery,
    /// Set when queued: the `queue_id` its `input/submitted` event will carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_id: Option<String>,
}

method!(
    /// Take a queued message back before it is delivered: to drop it, edit it, or
    /// steer it into the running turn instead. Returns its text.
    ThreadUnqueue, "thread.unqueue", ThreadUnqueueParams => ThreadUnqueueResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadUnqueueParams {
    pub thread_id: ThreadId,
    /// From `thread.send` or the `input/queued` event.
    pub queue_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadUnqueueResult {
    pub text: String,
}

method!(
    /// Change a thread's permission mode. Refused while a turn runs. An attached
    /// provider session is released; the next message resumes it in the new mode.
    ThreadSetMode, "thread.set_mode", ThreadSetModeParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetModeParams {
    pub thread_id: ThreadId,
    /// Absent: back to the agent's own configuration.
    #[serde(default)]
    pub permission_mode: Option<PermissionMode>,
}

method!(
    /// Turn an MCP server set up in qcc on or off for one thread. The agent's own
    /// servers are not qcc's to switch. The threads it starts are not affected.
    /// A provider that declared `mcp_toggle` switches it in the running session at
    /// once; for another the session is released as soon as no turn runs, so that
    /// the next message resumes it with the change.
    ThreadSetMcp, "thread.set_mcp", ThreadSetMcpParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetMcpParams {
    pub thread_id: ThreadId,
    pub server: String,
    pub enabled: bool,
}

method!(
    /// Go on with another agent, model or effort. Refused while a turn runs. An
    /// attached session is released; with the same provider the next message resumes
    /// it the new way, with another provider a new session starts and is told the
    /// conversation so far. A permission mode the new provider cannot enforce is
    /// dropped.
    ThreadSetAgent, "thread.set_agent", ThreadSetAgentParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetAgentParams {
    pub thread_id: ThreadId,
    pub provider: ProviderId,
    /// Absent: the agent's default model.
    #[serde(default)]
    pub model: Option<String>,
    /// Absent: the agent's default effort.
    #[serde(default)]
    pub effort: Option<String>,
}

method!(
    /// Interrupt the running turn or release the session. Returns once the provider
    /// answered and everything open is settled.
    ThreadStop, "thread.stop", ThreadStopParams => Empty
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum StopMode {
    /// Settle the running turn as interrupted; the session stays attached.
    #[default]
    Interrupt,
    /// Also detach the provider session. The thread can be resumed by sending input.
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadStopParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub mode: StopMode,
}

method!(ThreadList, "thread.list", ThreadListParams => ThreadListResult);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadListParams {
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadListResult {
    /// Oldest first.
    pub threads: Vec<Thread>,
}

method!(
    /// List every thread, archived ones included, and keep following them: whenever
    /// a thread appears or its projection changes, a [`THREAD_UPDATED`] notification
    /// carries the new projection, until [`ThreadUnsubscribe`] or the connection
    /// closes. Streamed text does not count as a change.
    ThreadWatch, "thread.watch", ThreadWatchParams => ThreadListResult
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadWatchParams {
    /// A client-chosen id, echoed in every notification.
    pub subscription: u64,
}

/// Notification: a watched thread appeared or changed.
pub const THREAD_UPDATED: &str = "thread.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadUpdatedNotification {
    pub subscription: u64,
    pub thread: Thread,
}

method!(ThreadGet, "thread.get", ThreadGetParams => ThreadResult);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadGetParams {
    pub thread_id: ThreadId,
}

method!(
    /// Put a thread away: release its session, remove its environment and leave it
    /// out of listings. Its timeline stays readable. Refused while a turn runs, and
    /// when removing the environment would lose work unless `force` is set.
    ThreadArchive, "thread.archive", ThreadArchiveParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadArchiveParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub force: bool,
}

method!(
    /// The files a thread changed in its environment.
    ThreadChanges, "thread.changes", ThreadGetParams => ThreadChangesResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadChangesResult {
    pub changes: EnvironmentChanges,
}

method!(
    /// Everything a thread changed in its environment, as one unified diff.
    ThreadDiff, "thread.diff", ThreadGetParams => ThreadDiffResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadDiffResult {
    pub diff: String,
    /// The diff was cut short because it is too large.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

method!(
    /// Bring what a thread did in its environment back to the directory the
    /// environment was made from: `apply` leaves it there uncommitted, `merge`
    /// commits it on the thread's branch and merges that branch. Refused, changing
    /// nothing, when it does not fit (conflicts) and while a turn runs.
    ThreadDeliver, "thread.deliver", ThreadDeliverParams => ThreadDeliverResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadDeliverParams {
    pub thread_id: ThreadId,
    pub mode: DeliverMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadDeliverResult {
    /// What was done, in a sentence.
    pub message: String,
}

method!(
    /// Move a thread that works in its directory into an environment of its own
    /// (a git worktree), between turns. With `carry_changes` its uncommitted work
    /// goes along and leaves the directory. The session is released; the next one
    /// starts in the new directory and is told the conversation.
    ThreadIsolate, "thread.isolate", ThreadIsolateParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadIsolateParams {
    pub thread_id: ThreadId,
    /// Id of a contributed environment (`git-worktree`).
    pub environment: String,
    #[serde(default)]
    pub carry_changes: bool,
}

method!(
    /// Wait until threads settle: nothing runs in them and nothing will until someone
    /// sends them a message (idle, failed or archived). A thread waiting for the user
    /// to answer an approval is not settled. Returns at once if the condition already
    /// holds.
    ThreadWait, "thread.wait", ThreadWaitParams => ThreadWaitResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadWaitParams {
    pub thread_ids: Vec<ThreadId>,
    /// Return when any of them settles instead of all.
    #[serde(default)]
    pub any: bool,
    /// Give up after this long and say so in `timed_out`. Absent: wait as long as
    /// it takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadWaitResult {
    /// In the order asked.
    pub threads: Vec<WaitedThread>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub timed_out: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WaitedThread {
    pub thread: Thread,
    pub state: ThreadState,
    /// The text of the agent's latest message: usually the answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
}

method!(
    /// Read a thread's events after a cursor and optionally keep following it.
    ThreadEvents, "thread.events", ThreadEventsParams => ThreadEventsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadEventsParams {
    pub thread_id: ThreadId,
    /// Return events with `seq > after`. Zero reads from the beginning.
    #[serde(default)]
    pub after: u64,
    /// A client-chosen id. When present, events appended later are pushed as
    /// [`THREAD_EVENT`] notifications carrying it, until [`ThreadUnsubscribe`] or
    /// the connection closes. The backlog and the live stream never have a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ThreadEventsResult {
    pub events: Vec<ThreadEvent>,
}

method!(ThreadUnsubscribe, "thread.unsubscribe", ThreadUnsubscribeParams => Empty);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadUnsubscribeParams {
    pub subscription: u64,
}

/// Notification: one event of a followed thread.
pub const THREAD_EVENT: &str = "thread.event";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ThreadEventNotification {
    pub subscription: u64,
    pub event: ThreadEvent,
}

method!(
    /// Give a thread a name of the user's choosing instead of its first message.
    ThreadRename, "thread.rename", ThreadRenameParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadRenameParams {
    pub thread_id: ThreadId,
    pub title: String,
}

method!(
    /// Put a thread into a project, move it to another or take it out (`None`).
    ThreadSetProject, "thread.set_project", ThreadSetProjectParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetProjectParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub project: Option<ProjectId>,
}

method!(
    /// Place a thread under another, or take it out to stand on its own (`None`).
    /// Refused when it would make a thread its own ancestor, or leave a thread
    /// running looser than the one it is placed under.
    ThreadSetParent, "thread.set_parent", ThreadSetParentParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetParentParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub parent: Option<ThreadId>,
}

// ----- projects ---------------------------------------------------------------------

method!(
    /// Every project, oldest first.
    ProjectList, "project.list", Empty => ProjectListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectListResult {
    pub projects: Vec<Project>,
}

/// Notification to connections that watch threads: the projects changed. Carries
/// all of them.
pub const PROJECTS_UPDATED: &str = "projects.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectsUpdatedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub projects: Vec<Project>,
}

method!(
    /// Make a project. Threads outside any project whose directory is in one of its
    /// folders join it.
    ProjectCreate, "project.create", ProjectCreateParams => ProjectResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectCreateParams {
    pub name: String,
    /// Absolute paths of existing directories: repositories or plain folders.
    pub folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectResult {
    pub project: Project,
}

method!(
    /// Rename a project or change its folders. Threads outside any project whose
    /// directory is in a new folder join it; threads already in it stay.
    ProjectUpdate, "project.update", ProjectUpdateParams => ProjectResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectUpdateParams {
    pub id: ProjectId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The whole new list. Absent: unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folders: Option<Vec<String>>,
}

method!(
    /// Delete a project. Its threads stay, outside any project; their files are not
    /// touched.
    ProjectDelete, "project.delete", ProjectDeleteParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectDeleteParams {
    pub id: ProjectId,
}

// ----- interactions -----------------------------------------------------------------

method!(
    /// Answer an approval or a question a provider is waiting on.
    InteractionResolve, "interaction.resolve", InteractionResolveParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct InteractionResolveParams {
    pub thread_id: ThreadId,
    pub interaction_id: InteractionId,
    pub resolution: InteractionResolution,
}

// ----- plugins ----------------------------------------------------------------------

method!(PluginList, "plugin.list", Empty => PluginListResult);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginListResult {
    pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    /// Shipped with the daemon.
    pub bundled: bool,
    /// An example or test plugin, not one for everyday use (`example` in its
    /// manifest). Clients hide what it contributes unless asked to show it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub example: bool,
    /// Permissions in effect: requested by the manifest and granted by the user.
    pub permissions: PluginPermissions,
    pub providers: Vec<ProviderInfo>,
    pub environments: Vec<EnvironmentInfo>,
    pub commands: Vec<CommandInfo>,
    /// Names of the skills it offers to agents.
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginPermissions {
    pub deltas: bool,
    pub kv: bool,
    pub timers: bool,
    pub threads: bool,
    pub skills: bool,
    /// Executables the plugin may run.
    pub process: Vec<String>,
}

// ----- providers --------------------------------------------------------------------

method!(
    /// The models a provider's agent offers. The first call asks the agent (which can
    /// take a few seconds); the answer is kept until `refresh` asks again.
    ProviderModels, "provider.models", ProviderModelsParams => ProviderModelsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderModelsParams {
    pub provider: ProviderId,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderModelsResult {
    /// In the agent's order. Empty when the agent cannot tell.
    pub models: Vec<ModelInfo>,
}

// ----- settings ---------------------------------------------------------------------

/// What the user set for the daemon. Kept in `<data>/settings.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Settings {
    /// The agent a new thread gets when none is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<ProviderId>,
    /// Where the programs plugins run live, by the name a plugin runs them by
    /// (`claude` → `/opt/claude/bin/claude`). A name not here is looked up on the
    /// daemon's `PATH`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub programs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub providers: BTreeMap<ProviderId, ProviderSettings>,
    /// MCP servers every agent session gets on top of those the agent's own
    /// configuration names, by server name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_servers: BTreeMap<String, McpServerSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct McpServerSettings {
    pub transport: McpTransport,
    /// Kept in the settings but given to no session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderSettings {
    /// The model a new thread gets when none is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The effort a new thread gets when none is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The permission mode a new thread gets when none is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    /// Variables every session of this agent sees (an API key, a config path).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Left out where the app offers agents to pick from.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A program some plugin may run, and which file that is now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProgramStatus {
    pub name: String,
    /// The plugins whose `process` permission names it.
    pub plugins: Vec<String>,
    /// The file that runs. Absent when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Why there is none, or why the configured one cannot run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SettingsResult {
    pub settings: Settings,
    /// Every program plugins may run, by name.
    pub programs: Vec<ProgramStatus>,
}

method!(SettingsGet, "settings.get", Empty => SettingsResult);

method!(
    /// Replace the settings. They apply at once: to programs started from now on and
    /// to threads spawned from now on.
    SettingsSet, "settings.set", SettingsSetParams => SettingsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SettingsSetParams {
    pub settings: Settings,
}

// ----- plugin commands --------------------------------------------------------------

method!(
    /// The commands plugins contribute.
    CommandList, "command.list", Empty => CommandListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandListResult {
    pub commands: Vec<CommandContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandContribution {
    pub plugin: String,
    #[serde(flatten)]
    pub command: CommandInfo,
}

method!(
    /// Run a plugin's command.
    CommandRun, "command.run", CommandRunParams => CommandRunResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandRunParams {
    pub plugin: String,
    pub command: String,
    /// Required by a command that acts on a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    /// Parameter values by name.
    #[serde(default)]
    pub args: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandRunResult {
    pub message: String,
    /// A thread the command made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

// ----- plugin UI --------------------------------------------------------------------

method!(
    /// The views and item renderers plugins contribute to the app.
    UiContributions, "ui.contributions", Empty => UiContributionsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiContributionsResult {
    pub views: Vec<UiViewContribution>,
    pub items: Vec<UiItemContribution>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiViewContribution {
    pub plugin: String,
    #[serde(flatten)]
    pub view: UiViewInfo,
    /// The providers the same plugin contributes: what `ownThreads` means.
    pub providers: Vec<ProviderId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiItemContribution {
    pub plugin: String,
    /// The extension item kind the plugin draws; always `<plugin>/<name>`.
    pub kind: String,
}

/// A tree of UI nodes. Each node is an object with a `type`; the app draws the
/// types it knows and skips the rest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct UiTreeResult {
    pub tree: Value,
}

method!(
    /// Ask a plugin to draw one of its views.
    UiRender, "ui.render", UiRenderParams => UiTreeResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiRenderParams {
    pub plugin: String,
    pub view: String,
    /// The thread a thread tab is shown for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
}

method!(
    /// Tell a plugin the user activated a node of its view. The answer replaces
    /// the view.
    UiAction, "ui.action", UiActionParams => UiTreeResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiActionParams {
    pub plugin: String,
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    pub action: String,
    /// What the node carried: a button's value, the text of an input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

method!(
    /// Ask a plugin to draw a timeline item of one of its extension kinds.
    UiRenderItem, "ui.render_item", UiRenderItemParams => UiTreeResult
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct UiRenderItemParams {
    pub plugin: String,
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

method!(
    /// Read one file of a plugin's web bundle.
    UiAsset, "ui.asset", UiAssetParams => UiAssetResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiAssetParams {
    pub plugin: String,
    /// Relative to the bundle: `index.html`, `assets/app.js`.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiAssetResult {
    /// The file, base64-encoded.
    pub data: String,
    pub content_type: String,
}
