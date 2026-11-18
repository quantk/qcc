//! Automations: prompts the daemon sends an agent on a schedule.

use serde::{Deserialize, Serialize};

use crate::ids::{AutomationId, ProviderId, ThreadId};
use crate::provider::PermissionMode;

/// Where each run of an automation goes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AutomationThreadMode {
    /// Every run is a thread of its own: runs do not see each other, and each one
    /// can be read, archived or delivered on its own.
    #[default]
    New,
    /// Every run is a message to the same thread, which the first run starts: the
    /// agent remembers earlier runs. A new thread is started when that one is
    /// archived or gone.
    Continue,
}

/// What happens when a run comes due while the previous one is still at work
/// (running a turn or waiting for the user).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AutomationOverlap {
    /// The run is skipped and recorded as such.
    #[default]
    Skip,
    /// The run goes ahead: a new thread runs alongside the previous one, or, in
    /// `continue` mode, the message waits in the thread's queue.
    Allow,
}

/// What the user decides about an automation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationSpec {
    pub name: String,
    /// When it runs: a cron expression of five fields (`0 9 * * 1-5`), one of
    /// `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`, or an interval
    /// (`every 30m`, `every 1h30m`, `every 2d`). Cron runs in the daemon's local time.
    pub schedule: String,
    /// The message each run sends the agent.
    pub prompt: String,
    pub provider: ProviderId,
    /// Absolute path of the directory the agent works in, or that its environment
    /// is made from.
    pub cwd: String,
    /// A contributed environment each new thread gets (`git-worktree`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Absent: the user's default for the agent, or else its own configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default)]
    pub thread_mode: AutomationThreadMode,
    #[serde(default)]
    pub overlap: AutomationOverlap,
    /// A run missed while the daemon was not running (or the computer slept) is
    /// made up for, once, as soon as the daemon notices. Absent: missed runs are
    /// recorded as skipped and the automation waits for its next time.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub catch_up: bool,
    pub enabled: bool,
}

/// An automation as the daemon keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Automation {
    pub id: AutomationId,
    #[serde(flatten)]
    pub spec: AutomationSpec,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    /// The thread `continue` mode sends its runs to, once the first run started it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    /// The scheduled time the daemon last dealt with (ran, skipped or found
    /// missed), or the time the schedule was last set or switched on. The next run
    /// is the first scheduled time after it.
    pub anchor: u64,
}

/// Why a run happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AutomationTrigger {
    /// Its scheduled time came.
    Schedule,
    /// Its scheduled time passed while the daemon was not running.
    Missed,
    /// The user asked for a run now.
    Manual,
}

/// How a run went, as far as starting it goes. How the agent's work went is the
/// thread's to tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AutomationOutcome {
    /// The prompt went to `thread_id`.
    Started,
    /// Nothing was sent; `reason` says why.
    Skipped,
    /// The thread could not be started; `reason` says why.
    Failed,
}

/// One time an automation came due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutomationRun {
    pub automation_id: AutomationId,
    /// When it was dealt with, in milliseconds since the Unix epoch.
    pub at: u64,
    /// The scheduled time it was for. Absent for a manual run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_for: Option<u64>,
    pub trigger: AutomationTrigger,
    pub outcome: AutomationOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
