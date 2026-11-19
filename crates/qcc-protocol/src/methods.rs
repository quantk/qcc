//! Methods and notifications of the daemon API.

use std::collections::BTreeMap;

use qcc_domain::{
    AgentSkill, Attachment, Automation, AutomationId, AutomationRun, AutomationSpec, ChangeScope,
    CommandInfo, CommentId, DeliverMode, DiffSide, EnvironmentChanges, EnvironmentInfo,
    InteractionId, InteractionResolution, LineComment, McpTransport, ModelInfo, PermissionMode,
    Project, ProjectIcon, ProjectId, ProviderId, ProviderInfo, ProviderLimits, SettingField,
    Thread, ThreadEvent, ThreadId, ThreadState, ToolBucket, UiViewInfo, UsageBucket,
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
    /// Where the environment makes the thread's directory: an absolute path.
    /// Absent: a place of the environment's own choosing. Only for an environment
    /// that declared `custom_placement`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_dir: Option<String>,
    /// What the environment calls the thread's branch. Absent: a name of its own.
    /// Only for an environment that declared `custom_placement`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// `branch` may exist already, and then the thread continues on it. Without
    /// this an existing branch is refused.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reuse_branch: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Files attached to `prompt`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<NewAttachment>,
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
    /// Skills offered on request that the thread gets (`skill.list`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_enabled: Vec<String>,
    /// Skills every session gets that the thread goes without.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_disabled: Vec<String>,
    /// The thread whose agent starts this one on the user's behalf. The new thread
    /// stands on its own (no `parent`) and is not held to that thread's mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_by: Option<ThreadId>,
    /// Kept out of clients' lists and overviews (`Thread::hidden`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
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
    /// Files attached to the message. The text may be empty when there are some.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<NewAttachment>,
}

/// A file to attach to a message: one on the daemon's machine (`path`) or its
/// contents (`data`), like a pasted image. qcc keeps a copy of it with the thread,
/// which the agent can read and the timeline shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NewAttachment {
    /// An absolute path. A file qcc already keeps for the thread (an attachment of
    /// a message taken back from the queue) is not copied again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file's contents, base64-encoded. Exactly one of `path` and `data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The file name to show. Absent: that of `path`, or one made up for `data`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Media type. Absent: told from the contents and the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

/// Something a user's message is about, which the agent is told along with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageContext {
    /// The user is looking at this thread.
    Thread { thread_id: ThreadId },
    /// Comments left on the thread's changes: the agent is told them in full.
    Comments { comment_ids: Vec<CommentId> },
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
    /// Its attachments, which can be sent again by `path`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

method!(
    /// Read a file attached to one of the thread's messages, to show it.
    ThreadAttachment, "thread.attachment", ThreadAttachmentParams => ThreadAttachmentResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadAttachmentParams {
    pub thread_id: ThreadId,
    /// `Attachment::path` as the thread's events give it.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadAttachmentResult {
    /// The file, base64-encoded.
    pub data: String,
    pub mime: String,
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
    /// A provider that declared `mcp_live` changes the running session at once; for
    /// another the session is released as soon as no turn runs, so that the next
    /// message resumes it with the change.
    ThreadSetMcp, "thread.set_mcp", ThreadSetMcpParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetMcpParams {
    pub thread_id: ThreadId,
    pub server: String,
    pub enabled: bool,
}

method!(
    /// Start the conversation afresh in the same thread: the next message starts a
    /// new session instead of resuming the one before. What came before stays in
    /// the journal as an earlier conversation; transcripts start after it. Refused
    /// while a turn runs.
    ThreadReset, "thread.reset", ThreadResetParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadResetParams {
    pub thread_id: ThreadId,
}

method!(
    /// Ask the agent to compact its context: to sum up the conversation so far and
    /// go on from the summary. Only for a provider that declared `compact`, and
    /// refused while a turn runs. The agent does it in a turn of its own, which
    /// ends with `session/compacted`; a thread without a session resumes one first.
    ThreadCompact, "thread.compact", ThreadCompactParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadCompactParams {
    pub thread_id: ThreadId,
}

method!(
    /// Turn a skill on or off in one thread. Off takes effect when the thread's next
    /// session starts: the agent may have read it already. On is told to a running
    /// session with the next message, so it need not start again.
    ThreadSetSkill, "thread.set_skill", ThreadSetSkillParams => ThreadResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadSetSkillParams {
    pub thread_id: ThreadId,
    pub skill: String,
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
    /// Only the threads in this state. `archived` lists the archived ones alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ThreadState>,
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
    ///
    /// With `descendants`, the threads started under it at any depth go too, deepest
    /// first, and the answer reports each of them. Working descendants are stopped
    /// on the way; the thread itself must not be working. A descendant that refuses
    /// keeps its ancestors (the thread included) out of the archive, so no archived
    /// thread is left with live threads under it; the other branches go on.
    ThreadArchive, "thread.archive", ThreadArchiveParams => ThreadArchiveResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadArchiveParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub force: bool,
    /// Archive the threads started under this one as well. Off by default, so a
    /// client that does not know about it archives the one thread, as before.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub descendants: bool,
}

/// The archived thread, and with `descendants` what happened to the threads under
/// it. Without `descendants` this is just `{ thread }`, and a refusal is an error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadArchiveResult {
    /// The thread asked for, as it is now: not archived if something under it, or
    /// it itself, refused.
    pub thread: Thread,
    /// Descendants archived by this call, deepest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archived: Vec<Thread>,
    /// Descendants that were working and were stopped to be archived.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stopped: Vec<ThreadId>,
    /// Threads of the tree, the thread asked for included, that refused and stay
    /// as they were.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refused: Vec<ArchiveRefusal>,
    /// Threads left alone because a thread under them refused.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kept: Vec<ThreadId>,
}

/// Why one thread of an archived tree stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ArchiveRefusal {
    pub thread_id: ThreadId,
    /// The error code archiving it alone would have answered with, such as
    /// `ENVIRONMENT_ERROR` for uncommitted work.
    pub code: i32,
    pub message: String,
}

method!(
    /// The files a thread changed in its environment, and the revisions it saved there.
    ThreadChanges, "thread.changes", ThreadChangesParams => ThreadChangesResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadChangesParams {
    pub thread_id: ThreadId,
    /// Which part of the work; all of it when absent.
    #[serde(default, skip_serializing_if = "ChangeScope::is_all")]
    pub scope: ChangeScope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadChangesResult {
    pub changes: EnvironmentChanges,
}

method!(
    /// Everything a thread changed in its environment, as one unified diff; with
    /// `paths`, only those files.
    ThreadDiff, "thread.diff", ThreadDiffParams => ThreadDiffResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadDiffParams {
    pub thread_id: ThreadId,
    /// Only these files, by their paths in `thread.changes`; every file when empty.
    /// A large change is read a few files at a time this way, and none of them is
    /// crowded out of a diff cut short by the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Which part of the work; all of it when absent.
    #[serde(default, skip_serializing_if = "ChangeScope::is_all")]
    pub scope: ChangeScope,
    /// Each changed file whole, its unchanged lines as context around the changes,
    /// instead of a few lines around each change. Meant for a file or two at a time:
    /// the diff of every file in full soon hits the size limit.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full: bool,
}

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
    /// As in `thread.spawn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_dir: Option<String>,
    /// As in `thread.spawn`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// As in `thread.spawn`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reuse_branch: bool,
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
    /// What every thread spent and how its turns went, hour by hour, oldest first,
    /// archived threads included. Hours with nothing in them are left out.
    UsageHistory, "usage.history", Empty => UsageHistoryResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UsageHistoryResult {
    /// How long each bucket is, in milliseconds.
    pub bucket_ms: u64,
    pub buckets: Vec<UsageBucket>,
}

method!(
    /// What every thread used — commands, tools, MCP tools, skills — how often, how
    /// the uses ended and how long they took, hour by hour, oldest first, archived
    /// threads included.
    UsageTools, "usage.tools", Empty => UsageToolsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UsageToolsResult {
    /// How long each bucket is, in milliseconds.
    pub bucket_ms: u64,
    pub buckets: Vec<ToolBucket>,
}

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
    /// Absent: no icon, the app draws its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProjectIcon>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProjectResult {
    pub project: Project,
}

method!(
    /// Rename a project, change its folders or its icon. Threads outside any project whose
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
    /// Absent: unchanged; `null`: no icon any more.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    #[specta(type = Option<ProjectIcon>)]
    pub icon: Option<Option<ProjectIcon>>,
}

/// A field that is there, `null` or not, as `Some`; with `#[serde(default)]` an
/// absent one stays `None`. This tells "leave it" from "clear it".
fn present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
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

// ----- automations ------------------------------------------------------------------

/// An automation and where it stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationEntry {
    pub automation: Automation,
    /// When it runs next, in milliseconds since the Unix epoch. Absent while it is
    /// switched off, and for a schedule with no time left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<u64>,
    /// The latest time it came due, whatever came of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run: Option<AutomationRun>,
}

method!(
    /// Every automation, oldest first.
    AutomationList, "automation.list", Empty => AutomationListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationListResult {
    pub automations: Vec<AutomationEntry>,
}

/// Notification to connections that watch threads: an automation was made,
/// changed, deleted or ran. Carries all of them.
pub const AUTOMATIONS_UPDATED: &str = "automations.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationsUpdatedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub automations: Vec<AutomationEntry>,
}

method!(
    /// Make an automation. The schedule, the provider, the directory, the
    /// environment and the permission mode are checked now, not at its first run.
    AutomationCreate, "automation.create", AutomationSpec => AutomationResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationResult {
    pub automation: AutomationEntry,
}

method!(
    /// Replace what an automation is. A new schedule, or switching it on, counts
    /// from now: runs it would have had before are not made up for.
    AutomationUpdate, "automation.update", AutomationUpdateParams => AutomationResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationUpdateParams {
    pub id: AutomationId,
    pub spec: AutomationSpec,
}

method!(
    /// Switch an automation on or off.
    AutomationSetEnabled, "automation.set_enabled", AutomationSetEnabledParams => AutomationResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationSetEnabledParams {
    pub id: AutomationId,
    pub enabled: bool,
}

method!(
    /// Delete an automation and its history. The threads it started stay.
    AutomationDelete, "automation.delete", AutomationIdParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationIdParams {
    pub id: AutomationId,
}

method!(
    /// Run an automation now, whether it is switched on or not. Its schedule goes
    /// on as before. A run that would overlap the previous one is skipped as a
    /// scheduled one would be; the answer says so.
    AutomationRunNow, "automation.run", AutomationIdParams => AutomationRunResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationRunResult {
    pub run: AutomationRun,
}

method!(
    /// An automation's latest runs, newest first.
    AutomationRuns, "automation.runs", AutomationRunsParams => AutomationRunsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationRunsParams {
    pub id: AutomationId,
    /// Absent: all that are kept (the latest 100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationRunsResult {
    pub runs: Vec<AutomationRun>,
}

// ----- comments ---------------------------------------------------------------------

method!(
    /// The comments left on a thread's changes, oldest first.
    CommentList, "comment.list", CommentListParams => CommentListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentListParams {
    pub thread_id: ThreadId,
    /// Resolved comments too. Absent: only the open ones.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub include_resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentListResult {
    pub comments: Vec<LineComment>,
}

method!(
    /// Leave a comment on a line of a thread's changes, for its agent to act on.
    CommentAdd, "comment.add", CommentAddParams => CommentResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentAddParams {
    pub thread_id: ThreadId,
    /// The file, as the thread's diff names it.
    pub path: String,
    /// 1-based, counted on `side`.
    pub line: u32,
    #[serde(default)]
    pub side: DiffSide,
    /// The text of the line, to find it by once the code moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentResult {
    pub comment: LineComment,
}

method!(
    /// Mark comments dealt with, saying what was done; or, with `reopen`, open them
    /// again. Every id must name a comment, or nothing changes.
    CommentResolve, "comment.resolve", CommentResolveParams => CommentListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentResolveParams {
    pub comment_ids: Vec<CommentId>,
    /// What was done about them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reopen: bool,
}

method!(
    /// Take a comment back.
    CommentDelete, "comment.delete", CommentDeleteParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentDeleteParams {
    pub comment_id: CommentId,
}

/// Notification to connections that watch threads: the comments on a thread's
/// changes changed. Carries all of them, resolved ones too.
pub const COMMENTS_UPDATED: &str = "comments.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommentsUpdatedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub thread_id: ThreadId,
    pub comments: Vec<LineComment>,
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

method!(
    /// Every installed plugin, the ones turned off included.
    PluginList, "plugin.list", Empty => PluginListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginListResult {
    pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What the plugin is for, from its manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub version: String,
    /// Shipped with the daemon.
    pub bundled: bool,
    /// Turned on. A plugin turned off is not running: it contributes nothing, and
    /// what it contributed is listed empty.
    pub enabled: bool,
    /// Turned on but could not be started: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    /// Settings of its own; their values are in `Settings::plugins`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingField>,
}

method!(
    /// Turn a plugin on and start it, at once and for later starts of the daemon.
    PluginEnable, "plugin.enable", PluginEnableParams => PluginResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginEnableParams {
    pub id: String,
}

method!(
    /// Turn a plugin off and stop it, at once and for later starts of the daemon.
    /// Its agents, environments, commands, skills and views go away; threads made
    /// with them stay readable but take no messages until it is back. Refused while
    /// one of its agents is working, unless `force`: then the turn is cut short.
    /// Idle sessions of its agents are closed either way.
    PluginDisable, "plugin.disable", PluginDisableParams => PluginResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginDisableParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginResult {
    pub plugin: PluginInfo,
}

/// Notification to connections that watch threads: a plugin was turned on or off,
/// so the agents, environments, commands and views on offer changed. Carries every
/// plugin, as `plugin.list` does.
pub const PLUGINS_UPDATED: &str = "plugins.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginsUpdatedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PluginPermissions {
    pub deltas: bool,
    pub kv: bool,
    pub timers: bool,
    pub threads: bool,
    pub skills: bool,
    /// Hears of every turn that ends.
    #[serde(default)]
    pub turn_events: bool,
    /// Leaves and reads comments on lines of threads' changes.
    #[serde(default)]
    pub comments: bool,
    /// Reads files in threads' working directories and its own folder.
    #[serde(default)]
    pub files: bool,
    /// Executables the plugin may run.
    pub process: Vec<String>,
}

// ----- skills -----------------------------------------------------------------------

method!(
    /// The skills plugins offer, and which of them the user turned off.
    SkillList, "skill.list", Empty => SkillListResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillListResult {
    /// By name.
    pub skills: Vec<SkillStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillStatus {
    pub name: String,
    pub description: String,
    /// The plugin that offers it.
    pub plugin: String,
    /// Given only to threads that ask for it, not to every session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_request: bool,
    /// Turned off in the settings: no session gets it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
    /// Where agents read it.
    pub path: String,
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

method!(
    /// The skills a provider's agent finds by itself (its own folders, its
    /// plugins), as the agent reports them. Shown, not switched. The first call asks
    /// the agent; the answer is kept until `refresh` asks again.
    ProviderSkills, "provider.skills", ProviderSkillsParams => ProviderSkillsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderSkillsParams {
    pub provider: ProviderId,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderSkillsResult {
    /// By name. Empty when the agent cannot tell.
    pub skills: Vec<AgentSkill>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderModelsResult {
    /// In the agent's order. Empty when the agent cannot tell.
    pub models: Vec<ModelInfo>,
}

method!(
    /// What the providers' accounts have spent of their subscriptions' usage limits,
    /// as last reported: by an agent during a session, or when asked. `refresh` asks
    /// the providers that report limits now (it runs their agents, without a
    /// prompt) and answers once they did.
    ProviderRateLimits, "provider.limits", ProviderLimitsParams => ProviderLimitsResult
);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderLimitsParams {
    /// Only this provider. Absent: every provider that reports limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderLimitsResult {
    /// By provider id; a provider whose limits were never reported is not listed.
    pub limits: Vec<ProviderLimits>,
    /// Providers a refresh could not read, with why. What they reported before
    /// stays in `limits`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<ProviderLimitsError>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderLimitsError {
    pub provider: ProviderId,
    pub message: String,
}

/// Notification to connections that watch threads: a provider reported its
/// account's usage limits. Carries what is known of that provider now.
pub const LIMITS_UPDATED: &str = "limits.updated";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LimitsUpdatedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub limits: ProviderLimits,
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
    /// configuration names, by server name. A change reaches running sessions the
    /// way `thread.set_mcp` does.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp_servers: BTreeMap<String, McpServerSettings>,
    /// What the user set for skills plugins offer, by skill name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skills: BTreeMap<String, SkillSettings>,
    /// The values of plugins' own settings (`PluginInfo::settings`): by plugin id,
    /// then by setting id. A setting not here has its default.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugins: BTreeMap<String, BTreeMap<String, String>>,
    /// Plugins turned off, by id, sorted. Changed only by `plugin.enable` and
    /// `plugin.disable`, which start and stop them: `settings.set` keeps it as it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_plugins: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillSettings {
    /// No session gets it, whatever a thread asks.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
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

/// Notification to connections that watch threads: a plugin's view shows something
/// else now and is worth asking for again (`ui.render`). For one thread's copy of
/// a thread view, or every copy when `thread_id` is absent.
pub const UI_CHANGED: &str = "ui.changed";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiChangedNotification {
    /// The subscription of the `thread.watch` it is sent for.
    pub subscription: u64,
    pub plugin: String,
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_project_update_tells_an_absent_icon_from_a_cleared_one() {
        let read =
            |value: Value| serde_json::from_value::<ProjectUpdateParams>(value).unwrap().icon;
        assert_eq!(read(json!({"id": "p"})), None);
        assert_eq!(read(json!({"id": "p", "icon": null})), Some(None));
        assert_eq!(
            read(json!({"id": "p", "icon": {"kind": "emoji", "emoji": "🚀"}})),
            Some(Some(ProjectIcon::Emoji { emoji: "🚀".into() }))
        );

        let write = |icon| {
            let params =
                ProjectUpdateParams { id: ProjectId::new("p"), name: None, folders: None, icon };
            serde_json::to_value(params).unwrap()
        };
        assert_eq!(write(None), json!({"id": "p"}));
        assert_eq!(write(Some(None)), json!({"id": "p", "icon": null}));
    }
}
