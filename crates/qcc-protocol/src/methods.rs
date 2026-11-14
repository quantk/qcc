//! Methods and notifications of the daemon API.

use qcc_domain::{
    EnvironmentInfo, InteractionId, InteractionResolution, ProviderId, ProviderInfo, Thread,
    ThreadEvent, ThreadId,
};
use serde::{Deserialize, Serialize};

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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Empty {}

// ----- daemon -----------------------------------------------------------------------

method!(
    /// Liveness and identity of the daemon.
    DaemonStatus, "daemon.status", Empty => DaemonStatusResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSpawnParams {
    pub provider: ProviderId,
    /// Working directory of the thread.
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The thread that spawned this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<ThreadId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadResult {
    pub thread: Thread,
}

method!(
    /// Send a user message. Starts or resumes the provider session if none is attached.
    ThreadSend, "thread.send", ThreadSendParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSendParams {
    pub thread_id: ThreadId,
    pub text: String,
}

method!(
    /// Interrupt the running turn or release the session. Returns once the provider
    /// answered and everything open is settled.
    ThreadStop, "thread.stop", ThreadStopParams => Empty
);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopMode {
    /// Settle the running turn as interrupted; the session stays attached.
    #[default]
    Interrupt,
    /// Also detach the provider session. The thread can be resumed by sending input.
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadStopParams {
    pub thread_id: ThreadId,
    #[serde(default)]
    pub mode: StopMode,
}

method!(ThreadList, "thread.list", Empty => ThreadListResult);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadListResult {
    /// Oldest first.
    pub threads: Vec<Thread>,
}

method!(ThreadGet, "thread.get", ThreadGetParams => ThreadResult);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadGetParams {
    pub thread_id: ThreadId,
}

method!(
    /// Read a thread's events after a cursor and optionally keep following it.
    ThreadEvents, "thread.events", ThreadEventsParams => ThreadEventsResult
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadEventsResult {
    pub events: Vec<ThreadEvent>,
}

method!(ThreadUnsubscribe, "thread.unsubscribe", ThreadUnsubscribeParams => Empty);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadUnsubscribeParams {
    pub subscription: u64,
}

/// Notification: one event of a followed thread.
pub const THREAD_EVENT: &str = "thread.event";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadEventNotification {
    pub subscription: u64,
    pub event: ThreadEvent,
}

// ----- interactions -----------------------------------------------------------------

method!(
    /// Answer an approval or a question a provider is waiting on.
    InteractionResolve, "interaction.resolve", InteractionResolveParams => Empty
);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InteractionResolveParams {
    pub thread_id: ThreadId,
    pub interaction_id: InteractionId,
    pub resolution: InteractionResolution,
}

// ----- plugins ----------------------------------------------------------------------

method!(PluginList, "plugin.list", Empty => PluginListResult);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginListResult {
    pub plugins: Vec<PluginInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub version: String,
    /// Shipped with the daemon.
    pub bundled: bool,
    /// Permissions in effect: requested by the manifest and granted by the user.
    pub permissions: PluginPermissions,
    pub providers: Vec<ProviderInfo>,
    pub environments: Vec<EnvironmentInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginPermissions {
    pub deltas: bool,
    pub kv: bool,
    pub timers: bool,
    /// Executables the plugin may run.
    pub process: Vec<String>,
}
