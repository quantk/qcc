use serde::{Deserialize, Serialize};

use crate::event::{EventPayload, ThreadEvent};
use crate::ids::{ProviderId, ThreadId};
use crate::provider::PermissionMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// The unit of work: one conversation with one provider in one working directory.
///
/// This is a projection: every field after creation changes only through
/// [`Thread::apply`], so replaying the journal rebuilds it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Put away: it takes no more input and is left out of listings.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub archived: bool,
}

fn is_zero(count: &u32) -> bool {
    *count == 0
}

const TITLE_MAX_CHARS: usize = 80;

impl Thread {
    /// Fold one event into the projection.
    pub fn apply(&mut self, event: &ThreadEvent) {
        self.updated_at = event.ts;
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
            }
            EventPayload::SessionIdentified { provider_session } => {
                self.provider_session = Some(provider_session.clone());
            }
            EventPayload::InputQueued { .. } => self.queued_inputs += 1,
            EventPayload::InputSubmitted { queue_id: Some(_), .. }
            | EventPayload::InputDropped { .. } => {
                self.queued_inputs = self.queued_inputs.saturating_sub(1);
            }
            EventPayload::PermissionModeChanged { permission_mode } => {
                self.permission_mode = *permission_mode;
            }
            EventPayload::EnvironmentPreparing => self.set_environment(EnvironmentState::Pending),
            EventPayload::EnvironmentReady { cwd } => {
                self.cwd.clone_from(cwd);
                self.set_environment(EnvironmentState::Ready);
            }
            EventPayload::EnvironmentFailed { .. } => {
                self.set_environment(EnvironmentState::Failed);
            }
            EventPayload::EnvironmentRemoved => self.set_environment(EnvironmentState::Removed),
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
