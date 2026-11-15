use serde::{Deserialize, Serialize};

use crate::event::{EventPayload, ThreadEvent};
use crate::ids::{ProviderId, ThreadId};

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

/// The unit of work: one conversation with one provider in one working directory.
///
/// This is a projection: every field after creation changes only through
/// [`Thread::apply`], so replaying the journal rebuilds it exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub provider: ProviderId,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
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
}

const TITLE_MAX_CHARS: usize = 80;

impl Thread {
    /// Fold one event into the projection.
    pub fn apply(&mut self, event: &ThreadEvent) {
        self.updated_at = event.ts;
        self.last_seq = event.seq;
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
            EventPayload::InputSubmitted { text } if self.title.is_none() => {
                let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
                if !line.is_empty() {
                    self.title = Some(line.chars().take(TITLE_MAX_CHARS).collect());
                }
            }
            _ => {}
        }
    }
}
