use serde::{Deserialize, Serialize};

use crate::event::{EventPayload, ThreadEvent};
use crate::ids::{InteractionId, ProjectId, ProviderId, ThreadId};
use crate::interaction::InteractionRequest;
use crate::item::{Item, ItemShape, ItemStatus};
use crate::provider::{McpServerStatus, PermissionMode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ThreadStatus {
    /// No turn is running; the thread accepts input.
    Idle,
    /// A turn is running.
    Active,
    /// The provider session could not be started or died with an error.
    Failed,
}

/// Where a thread's environment is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum EnvironmentState {
    /// Not prepared yet, or being prepared.
    Pending,
    /// The thread works in the directory the environment made.
    Ready,
    /// Preparation failed; the next message tries again.
    Failed,
    /// The directory was removed. The thread cannot run any more.
    Removed,
}

/// The working directory a thread asked an environment plugin to make for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadEnvironment {
    /// Id of the contributed environment.
    pub id: String,
    /// The directory the thread was asked to work on. Its own `cwd` points into
    /// the environment once that is ready.
    pub source_dir: String,
    /// What the environment starts from; its meaning belongs to the environment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub state: EnvironmentState,
    /// The thread worked in `source_dir` before and moves in: its uncommitted work
    /// goes along.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub carry_changes: bool,
}

/// Where a thread is, as far as someone waiting on it cares. Derived by the core
/// from the projection and what is in flight; never journaled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ThreadState {
    /// A turn runs, a message waits for one, or the session is starting.
    Working,
    /// The agent waits for the user to answer an approval or a question.
    Waiting,
    /// Nothing runs; the thread takes input.
    Idle,
    /// The provider session could not start or died.
    Failed,
    Archived,
}

impl ThreadState {
    /// Nothing will happen in the thread until someone sends it something.
    pub fn is_settled(self) -> bool {
        matches!(self, Self::Idle | Self::Failed | Self::Archived)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    User,
    Agent,
}

/// One message of a conversation: what the user sent or what the agent replied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct TranscriptEntry {
    pub role: Role,
    pub text: String,
}

/// An approval or a question the agent is waiting on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PendingInteraction {
    pub id: InteractionId,
    pub request: InteractionRequest,
}

/// What kind of thing an agent last did. Mirrors the item shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ActivityKind {
    AgentMessage,
    Reasoning,
    Plan,
    Command,
    FileChange,
    FileRead,
    Search,
    Tool,
    Extension,
}

/// The latest item of a thread, cut down to one line: enough to tell at a glance
/// what the agent is doing without reading its timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ThreadActivity {
    pub kind: ActivityKind,
    /// The command, path, query or tool name; for text, its first line once the
    /// item has settled. Never longer than [`ACTIVITY_MAX_CHARS`] characters.
    pub detail: String,
    pub status: ItemStatus,
}

pub const ACTIVITY_MAX_CHARS: usize = 200;

impl ThreadActivity {
    fn of(item: &Item) -> Self {
        let (kind, detail) = match &item.shape {
            ItemShape::AgentMessage { text } => (ActivityKind::AgentMessage, first_line(text)),
            ItemShape::Reasoning { text } => (ActivityKind::Reasoning, first_line(text)),
            ItemShape::Plan { text } => (ActivityKind::Plan, first_line(text)),
            ItemShape::Command { command, .. } => (ActivityKind::Command, first_line(command)),
            ItemShape::FileChange { path, .. } => (ActivityKind::FileChange, first_line(path)),
            ItemShape::FileRead { path } => (ActivityKind::FileRead, first_line(path)),
            ItemShape::Search { query } => (ActivityKind::Search, first_line(query)),
            ItemShape::Tool { name, .. } => (ActivityKind::Tool, first_line(name)),
            ItemShape::Extension { kind, .. } => (ActivityKind::Extension, first_line(kind)),
        };
        Self { kind, detail, status: item.status }
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    line.chars().take(ACTIVITY_MAX_CHARS).collect()
}

/// The unit of work: one conversation with one provider in one working directory.
///
/// This is a projection: every field after creation changes only through
/// [`Thread::apply`], so replaying the journal rebuilds it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Thread {
    pub id: ThreadId,
    pub provider: ProviderId,
    /// Where the agent works. For a thread with an environment this is the source
    /// directory until the environment is ready.
    pub cwd: String,
    /// Absent: the thread works directly in `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<ThreadEnvironment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
    /// The project the thread is grouped under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    /// Absent: the agent's own configuration decides what it may do unasked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub status: ThreadStatus,
    /// The provider's durable session handle, used to resume after a restart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session: Option<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    pub updated_at: u64,
    /// Sequence number of the last event applied.
    pub last_seq: u64,
    /// Messages waiting for the running turn to end.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub queued_inputs: u32,
    /// Approvals and questions the agent is waiting on, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_interactions: Vec<PendingInteraction>,
    /// The latest item, in one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<ThreadActivity>,
    /// When the running turn started, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_started_at: Option<u64>,
    /// How the latest turn ended. Absent while one runs and before the first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_status: Option<TurnStatus>,
    /// When the latest turn ended, in milliseconds since the Unix epoch: what a
    /// client compares with what the user has seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_at: Option<u64>,
    /// The MCP servers of the current session as its agent reports them, in the
    /// order they were first reported. Empty until it says and for agents that do not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerStatus>,
    /// MCP servers set up in qcc that this thread keeps off, by name, sorted: its
    /// sessions start without them. Not passed on to the threads it starts.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_disabled: Vec<String>,
    /// Put away: it takes no more input and is left out of listings.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
    /// The provider changed since the last session started. The next session has
    /// nothing to resume and is told the conversation so far instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub handover: bool,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

const TITLE_MAX_CHARS: usize = 80;

impl Thread {
    /// Fold one event into the projection.
    pub fn apply(&mut self, event: &ThreadEvent) {
        // Filing or naming a thread is not something it did, nor is its MCP
        // servers connecting.
        if !matches!(
            event.payload,
            EventPayload::ProjectChanged { .. }
                | EventPayload::TitleChanged { .. }
                | EventPayload::ParentChanged { .. }
                | EventPayload::McpServersChanged { .. }
        ) {
            self.updated_at = event.ts;
        }
        self.last_seq = event.seq;
        // The first message names the thread, whether it was sent or queued.
        if self.title.is_none()
            && let EventPayload::InputSubmitted { text, .. }
            | EventPayload::InputQueued { text, .. } = &event.payload
        {
            let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            if !line.is_empty() {
                self.title = Some(line.chars().take(TITLE_MAX_CHARS).collect());
            }
        }
        match &event.payload {
            EventPayload::ThreadStatusChanged { status } => self.status = *status,
            EventPayload::SessionStarted { provider_session, .. } => {
                if provider_session.is_some() {
                    self.provider_session.clone_from(provider_session);
                }
                self.handover = false;
            }
            EventPayload::ProjectChanged { project } => self.project.clone_from(project),
            EventPayload::ParentChanged { parent } => self.parent.clone_from(parent),
            EventPayload::TitleChanged { title } => self.title = Some(title.clone()),
            EventPayload::AgentChanged { provider, model } => {
                if *provider != self.provider {
                    self.provider = provider.clone();
                    self.provider_session = None;
                    self.handover = true;
                }
                self.model.clone_from(model);
            }
            EventPayload::SessionIdentified { provider_session } => {
                self.provider_session = Some(provider_session.clone());
            }
            EventPayload::InputQueued { .. } => self.queued_inputs += 1,
            EventPayload::InputSubmitted { queue_id: Some(_), .. }
            | EventPayload::InputDropped { .. } => {
                self.queued_inputs = self.queued_inputs.saturating_sub(1);
            }
            EventPayload::InteractionRequested { interaction_id, request, .. } => {
                self.pending_interactions.push(PendingInteraction {
                    id: interaction_id.clone(),
                    request: request.clone(),
                });
            }
            EventPayload::InteractionResolved { interaction_id, .. } => {
                self.pending_interactions.retain(|pending| &pending.id != interaction_id);
            }
            EventPayload::TurnStarted { .. } => {
                self.turn_started_at = Some(event.ts);
                self.last_turn_status = None;
            }
            EventPayload::TurnCompleted { status, .. } => {
                self.turn_started_at = None;
                self.last_turn_status = Some(*status);
                self.last_turn_at = Some(event.ts);
            }
            EventPayload::ItemStarted { item, .. } | EventPayload::ItemCompleted { item, .. } => {
                self.activity = Some(ThreadActivity::of(item));
            }
            EventPayload::PermissionModeChanged { permission_mode } => {
                self.permission_mode = *permission_mode;
            }
            EventPayload::EnvironmentAttached { environment } => {
                self.environment = Some(environment.clone());
                // The session lives in the old directory; the next one starts in
                // the new one and is told the conversation.
                if self.provider_session.take().is_some() {
                    self.handover = true;
                }
            }
            EventPayload::EnvironmentDetached { .. } => self.environment = None,
            EventPayload::EnvironmentPreparing => self.set_environment(EnvironmentState::Pending),
            EventPayload::EnvironmentReady { cwd } => {
                self.cwd.clone_from(cwd);
                self.set_environment(EnvironmentState::Ready);
            }
            EventPayload::EnvironmentFailed { .. } => {
                self.set_environment(EnvironmentState::Failed);
            }
            EventPayload::EnvironmentRemoved => self.set_environment(EnvironmentState::Removed),
            EventPayload::McpServersChanged { servers } => self.mcp_servers.clone_from(servers),
            EventPayload::McpDisabledChanged { disabled } => self.mcp_disabled.clone_from(disabled),
            EventPayload::ThreadArchived => self.archived = true,
            _ => {}
        }
    }

    fn set_environment(&mut self, state: EnvironmentState) {
        if let Some(environment) = &mut self.environment {
            environment.state = state;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ItemId, TurnId};
    use crate::interaction::InteractionResolution;

    fn thread() -> Thread {
        Thread {
            id: ThreadId::new("t"),
            provider: ProviderId::new("echo"),
            cwd: "/work".into(),
            environment: None,
            model: None,
            parent: None,
            project: None,
            permission_mode: None,
            title: None,
            status: ThreadStatus::Idle,
            provider_session: None,
            created_at: 0,
            updated_at: 0,
            last_seq: 0,
            queued_inputs: 0,
            pending_interactions: Vec::new(),
            mcp_servers: Vec::new(),
            mcp_disabled: Vec::new(),
            activity: None,
            turn_started_at: None,
            last_turn_status: None,
            last_turn_at: None,
            archived: false,
            handover: false,
        }
    }

    fn apply(thread: &mut Thread, payloads: Vec<EventPayload>) {
        for payload in payloads {
            let seq = thread.last_seq + 1;
            thread.apply(&ThreadEvent { thread_id: thread.id.clone(), seq, ts: seq * 10, payload });
        }
    }

    #[test]
    fn the_projection_says_what_the_agent_is_doing() {
        let mut thread = thread();
        let turn = TurnId::new("u");
        let command = |status| Item {
            id: ItemId::new("i"),
            status,
            shape: ItemShape::Command {
                command: "cargo test\n--workspace".into(),
                cwd: None,
                output: None,
                exit_code: None,
            },
        };
        apply(
            &mut thread,
            vec![
                EventPayload::TurnStarted { turn_id: turn.clone() },
                EventPayload::ItemStarted {
                    turn_id: turn.clone(),
                    item: command(ItemStatus::Pending),
                },
            ],
        );
        assert_eq!(thread.turn_started_at, Some(10));
        assert_eq!(
            thread.activity,
            Some(ThreadActivity {
                kind: ActivityKind::Command,
                detail: "cargo test".into(),
                status: ItemStatus::Pending,
            })
        );

        let request =
            InteractionRequest::Approval { title: "Run?".into(), detail: None, options: vec![] };
        apply(
            &mut thread,
            vec![EventPayload::InteractionRequested {
                interaction_id: InteractionId::new("x"),
                turn_id: None,
                item_id: None,
                request: request.clone(),
            }],
        );
        assert_eq!(
            thread.pending_interactions,
            vec![PendingInteraction { id: InteractionId::new("x"), request }]
        );

        apply(
            &mut thread,
            vec![
                EventPayload::InteractionResolved {
                    interaction_id: InteractionId::new("x"),
                    resolution: InteractionResolution::Cancelled,
                },
                EventPayload::ItemCompleted {
                    turn_id: turn.clone(),
                    item: command(ItemStatus::Failed),
                },
                EventPayload::TurnCompleted {
                    turn_id: turn,
                    status: TurnStatus::Failed,
                    error: None,
                },
            ],
        );
        assert!(thread.pending_interactions.is_empty());
        assert_eq!(
            thread.activity.as_ref().map(|activity| activity.status),
            Some(ItemStatus::Failed)
        );
        assert_eq!(thread.turn_started_at, None);
        assert_eq!(thread.last_turn_status, Some(TurnStatus::Failed));
        assert_eq!(thread.last_turn_at, Some(thread.updated_at));
    }

    #[test]
    fn another_agent_starts_afresh_and_is_handed_the_conversation() {
        let mut thread = thread();
        thread.provider_session = Some("s".into());
        let model = |model: &str| EventPayload::AgentChanged {
            provider: ProviderId::new("echo"),
            model: Some(model.into()),
        };
        apply(&mut thread, vec![model("big")]);
        // Another model of the same agent resumes the same session.
        assert_eq!(thread.model.as_deref(), Some("big"));
        assert_eq!(thread.provider_session.as_deref(), Some("s"));
        assert!(!thread.handover);

        apply(
            &mut thread,
            vec![EventPayload::AgentChanged { provider: ProviderId::new("other"), model: None }],
        );
        assert_eq!(thread.provider, ProviderId::new("other"));
        assert_eq!(thread.model, None);
        assert_eq!(thread.provider_session, None);
        assert!(thread.handover);

        apply(
            &mut thread,
            vec![EventPayload::SessionStarted { provider_session: None, resumed: false }],
        );
        assert!(!thread.handover);
    }

    #[test]
    fn activity_is_one_short_line() {
        let long = "x".repeat(ACTIVITY_MAX_CHARS * 2);
        let item = Item {
            id: ItemId::new("i"),
            status: ItemStatus::Completed,
            shape: ItemShape::AgentMessage { text: format!("\n\n  {long}\nsecond line") },
        };
        let activity = ThreadActivity::of(&item);
        assert_eq!(activity.detail.chars().count(), ACTIVITY_MAX_CHARS);
        assert!(activity.detail.chars().all(|c| c == 'x'));
    }
}
