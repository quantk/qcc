//! The core task: the single owner of all daemon state.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use qcc_domain::{
    AgentSkill, Automation, Delta, EventPayload, ModelInfo, Project, ProviderId, ProviderInfo,
    ProviderLimits, RateLimits, ThreadEnvironment, ThreadEvent, ThreadId, ThreadStatus, UserInput,
};
use qcc_plugin_host::{
    EnvironmentRequest, Outcome, PluginError, PluginHandle, PluginHost, PluginRequest,
    ProgramPaths, ThreadsAnswer, ThreadsCall,
};
use qcc_protocol::Method;
use qcc_protocol::jsonrpc::{Request, Response, RpcError, codes};
use qcc_protocol::methods::{
    Settings, THREAD_EVENT, THREAD_UPDATED, ThreadEventNotification, ThreadResult,
    ThreadUpdatedNotification,
};
use qcc_runtime::{Supervisor, UuidMint};
use qcc_store::Store;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{Config, plugins, settings};

mod attachments;
mod automations;
mod comments;
mod configuration;
mod environments;
mod extensions;
mod limits;
mod rpc;
mod sessions;
mod skills;
mod threads;
mod toggle;
mod waits;

/// Answers one request on the connection it came from.
pub(crate) struct Responder {
    out: UnboundedSender<String>,
    /// Absent for notifications, which get no answer.
    id: Option<Value>,
}

impl Responder {
    pub(crate) fn new(out: UnboundedSender<String>, id: Option<Value>) -> Self {
        Self { out, id }
    }

    fn reply<T: Serialize>(self, result: Result<T, RpcError>) {
        let Some(id) = self.id else { return };
        let response = match result.and_then(|value| {
            serde_json::to_value(value)
                .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))
        }) {
            Ok(value) => Response::ok(id, value),
            Err(error) => Response::err(id, error),
        };
        let _ = self.out.send(serde_json::to_string(&response).expect("responses serialize"));
    }
}

/// What a finished plugin call was for.
pub(crate) enum Action {
    Input,
    Steer,
    Resolve,
    Interrupt,
    Release,
    /// Change the qcc MCP servers of the running session.
    UpdateMcp,
    /// Compact the session's context, in a turn of the provider's own.
    Compact,
    /// Release, then start the conversation afresh: the old session's end belongs
    /// to the old conversation.
    Reset,
    /// Release, as the first step of archiving the thread.
    Archive {
        force: bool,
    },
}

/// A worktree may have dependencies to install before it is ready.
const PREPARE_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const REMOVE_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const QUERY_TIMEOUT: Duration = Duration::from_secs(60);
/// A provider lists its models by running its agent; past this it is not coming.
const MODELS_TIMEOUT: Duration = Duration::from_secs(30);

/// How far below a thread the user started agents may spawn agents: a guard
/// against an orchestrator that keeps delegating.
pub(crate) const MAX_SPAWN_DEPTH: usize = 4;

/// Ask an environment plugin, and stop waiting if it never answers.
async fn call_environment(
    plugin: PluginHandle,
    environment: String,
    request: EnvironmentRequest,
    limit: Duration,
) -> Result<Outcome, PluginError> {
    let request = PluginRequest::Environment { environment, request };
    match tokio::time::timeout(limit, plugin.call(request)).await {
        Ok(result) => result,
        Err(_) => Err(PluginError::Rejected(format!(
            "the environment did not answer within {} s",
            limit.as_secs()
        ))),
    }
}

/// Whether an extension item kind belongs to a plugin: `<plugin-id>/<name>`.
fn owns_kind(plugin: &str, kind: &str) -> bool {
    kind.strip_prefix(plugin).is_some_and(|rest| rest.len() > 1 && rest.starts_with('/'))
}

fn environment_error(message: impl Into<String>) -> RpcError {
    RpcError::new(codes::ENVIRONMENT_ERROR, message)
}

pub(crate) enum Command {
    Rpc {
        connection: u64,
        method: String,
        params: Value,
        responder: Responder,
    },
    Disconnected {
        connection: u64,
    },
    /// A plugin emitted deltas.
    Deltas {
        plugin: String,
        thread: ThreadId,
        deltas: Vec<Delta>,
    },
    PluginCrashed {
        plugin: String,
        reason: String,
    },
    SessionStartFinished {
        thread: ThreadId,
        resumed: bool,
        result: Result<Outcome, PluginError>,
    },
    CallFinished {
        thread: ThreadId,
        action: Action,
        result: Result<Outcome, PluginError>,
        responder: Option<Responder>,
    },
    /// The environment plugin answered `prepare`. `prompt` is the message that is
    /// waiting for the working directory.
    EnvironmentPrepared {
        thread: ThreadId,
        prompt: Option<UserInput>,
        result: Result<Outcome, PluginError>,
    },
    /// The environment plugin answered `remove` for a thread being archived.
    EnvironmentRemoved {
        thread: ThreadId,
        result: Result<Outcome, PluginError>,
        responder: Responder,
    },
    /// A plugin's `threads` import. Answered on `reply`, which the plugin's thread
    /// blocks on.
    Threads {
        plugin: String,
        call: ThreadsCall,
        /// The plugin stops waiting soon after; a call this late is not run.
        deadline: Instant,
        reply: std::sync::mpsc::Sender<Result<ThreadsAnswer, String>>,
    },
    /// A provider answered `list-models`.
    ModelsListed {
        provider: ProviderId,
        result: Result<Vec<ModelInfo>, String>,
        responder: Responder,
    },
    SkillsListed {
        provider: ProviderId,
        result: Result<Vec<AgentSkill>, String>,
        responder: Responder,
    },
    /// The providers asked by `provider.limits` answered (or gave up).
    LimitsRead {
        only: Option<ProviderId>,
        results: Vec<(ProviderId, Result<RateLimits, String>)>,
        responder: Responder,
    },
    /// A `thread.wait` ran out of time.
    WaitTimedOut {
        waiter: u64,
    },
    /// The automations' timer went off. Only the latest timer set counts.
    AutomationTick {
        generation: u64,
    },
    Shutdown,
}

/// A `thread.wait` that has not been answered yet.
struct Waiter {
    id: u64,
    connection: u64,
    threads: Vec<ThreadId>,
    any: bool,
    responder: Responder,
}

/// A plugin asked to hear when a thread settles.
struct Watch {
    plugin: String,
    thread: ThreadId,
}

/// Per-thread state that does not outlive the daemon process.
#[derive(Default)]
struct Session {
    /// The provider was asked to start and has not answered yet.
    starting: bool,
    /// The provider was asked to interrupt the turn or release the session and has
    /// not answered yet. Input meanwhile waits in the queue: delivered now, it could
    /// reach a session being torn down, or start a turn the answer then settles.
    stopping: bool,
    /// Deltas that arrived before the start was answered. They are applied after
    /// `session/started`, so the timeline reads in causal order.
    buffered: Vec<Delta>,
    /// An environment the thread moves into once its session is released: prepared
    /// earlier, the old session's last deltas would be taken for the new one's.
    prepare_after_release: Option<ThreadEnvironment>,
    /// The thread's MCP servers changed and its provider takes that only in a new
    /// session: release this one as soon as no turn runs.
    release_when_idle: bool,
    /// The provider is changing the session's MCP servers and has not answered yet.
    mcp_updating: bool,
    /// The thread's MCP servers changed while the session could not take it (it was
    /// starting, stopping or taking an earlier change): send them once it can.
    mcp_stale: bool,
    /// What qcc has to tell the agent with its next message: news since the
    /// session started. A new session is told everything anyway and drops them.
    notes: Vec<String>,
    /// The user asked to compact the context of a thread that had no session: ask
    /// once the one being started is up.
    compact_after_start: bool,
}

struct Subscriber {
    connection: u64,
    subscription: u64,
    out: UnboundedSender<String>,
}

pub(crate) struct Core {
    config: Config,
    commands: UnboundedSender<Command>,
    store: Store,
    supervisor: Supervisor,
    host: PluginHost,
    /// Every plugin in the plugin directory, running or not.
    installed: Vec<plugins::Installed>,
    /// Why plugins that should run could not be started, by id.
    load_errors: HashMap<String, String>,
    sessions: HashMap<ThreadId, Session>,
    /// Threads with input waiting for their running turn to end.
    queued: HashSet<ThreadId>,
    /// Threads moving into an environment: if preparing it fails they stay where
    /// they were instead of failing.
    isolating: HashSet<ThreadId>,
    /// Threads on their way to the archive. They take no input meanwhile.
    archiving: HashSet<ThreadId>,
    subscribers: HashMap<ThreadId, Vec<Subscriber>>,
    /// Connections following the list of threads.
    watchers: Vec<Subscriber>,
    waiters: Vec<Waiter>,
    next_waiter: u64,
    watches: Vec<Watch>,
    /// Skills plugins offer, written out where agents can read them.
    skills: Vec<plugins::OfferedSkill>,
    /// Oldest first.
    projects: Vec<Project>,
    /// Oldest first.
    automations: Vec<Automation>,
    /// Counts the timers set for automations; a tick of an earlier one is ignored.
    automation_timer: u64,
    settings: Settings,
    /// Where the programs plugins run live, as `settings.programs` says.
    programs: ProgramPaths,
    /// What each provider said it offers, until asked to look again.
    models: HashMap<ProviderId, Vec<ModelInfo>>,
    /// The skills each provider's agent said it finds by itself, until asked again.
    agent_skills: HashMap<ProviderId, Vec<AgentSkill>>,
    /// The latest usage limits each provider reported, saved in the store.
    limits: BTreeMap<ProviderId, ProviderLimits>,
    /// Where the `qcc` CLI lives, put first on agents' `PATH` so they can use it.
    cli_dir: Option<PathBuf>,
    /// Set when the journal cannot be written. The daemon stops rather than let
    /// memory and disk disagree.
    fatal: Option<anyhow::Error>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn params<M: Method>(params: Value) -> Result<M::Params, RpcError> {
    // Methods without parameters may omit them.
    let params = if params.is_null() { Value::Object(Default::default()) } else { params };
    serde_json::from_value(params)
        .map_err(|error| RpcError::new(codes::INVALID_PARAMS, format!("{}: {error}", M::NAME)))
}

/// Send a notification on a connection. False once the connection is gone, so
/// callers can drop it from their lists.
fn notify(out: &UnboundedSender<String>, method: &str, params: impl Serialize) -> bool {
    let params = serde_json::to_value(params).expect("notifications serialize");
    let line = serde_json::to_string(&Request::notification(method, params))
        .expect("notifications serialize");
    out.send(line).is_ok()
}

fn thread_not_found(id: &ThreadId) -> RpcError {
    RpcError::new(codes::THREAD_NOT_FOUND, format!("thread {id} does not exist"))
}

impl Core {
    /// The plugin that provides `provider`, and what it says about it.
    fn provider_of(
        &self,
        provider: &ProviderId,
    ) -> Result<(&PluginHandle, &ProviderInfo), RpcError> {
        self.host.provider(provider).ok_or_else(|| self.provider_missing(provider))
    }

    /// The thread as it is now, as most thread methods answer.
    fn thread_result(&self, id: &ThreadId) -> Result<ThreadResult, RpcError> {
        let thread = self.supervisor.thread(id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(id))
    }

    /// Open the journal, load the plugins and adopt the persisted threads.
    pub(crate) fn start(
        config: &Config,
        commands: UnboundedSender<Command>,
    ) -> anyhow::Result<Self> {
        let store = Store::open(config.database())
            .with_context(|| format!("cannot open {}", config.database().display()))?;
        let settings = settings::load(&config.settings());
        let programs = ProgramPaths::default();
        programs.set(settings.programs.iter().map(|(name, path)| (name.clone(), path.into())));
        let installed = plugins::install(config).context("cannot install plugins")?;
        let (host, load_errors) =
            plugins::load_all(config, &installed, &settings.disabled_plugins, &commands, &programs)
                .context("cannot load plugins")?;
        let skills = plugins::write_skills(config, &host);
        let cli_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
            .filter(|dir| dir.join("qcc").is_file());

        let projects = store.projects()?;
        let automations = store.automations()?;
        let limits = store
            .provider_limits()?
            .into_iter()
            .map(|limits| (limits.provider.clone(), limits))
            .collect();
        let mut supervisor = Supervisor::new(|| Box::new(UuidMint));
        let mut queued = HashSet::new();
        let now = now_ms();
        for thread in store.threads()? {
            // Only a thread that was mid-turn has anything open to settle, and only one
            // with queued messages has a queue to rebuild.
            let history = if thread.status == ThreadStatus::Active || thread.queued_inputs > 0 {
                store.events(&thread.id, 0)?
            } else {
                Vec::new()
            };
            let id = thread.id.clone();
            let settled = supervisor.recover(thread, history.iter().map(|e| &e.payload), now);
            if !settled.is_empty() {
                tracing::info!(thread = %id, "settled a turn interrupted by the last shutdown");
                store.append(&settled)?;
            }
            // Messages that were waiting behind the interrupted turn are still owed.
            if supervisor.engine(&id).is_ok_and(|engine| engine.has_queued_input()) {
                queued.insert(id);
            }
        }

        let core = Self {
            config: config.clone(),
            commands,
            store,
            supervisor,
            host,
            installed,
            load_errors,
            sessions: HashMap::new(),
            queued,
            archiving: HashSet::new(),
            isolating: HashSet::new(),
            subscribers: HashMap::new(),
            watchers: Vec::new(),
            waiters: Vec::new(),
            next_waiter: 0,
            watches: Vec::new(),
            skills,
            models: HashMap::new(),
            agent_skills: HashMap::new(),
            limits,
            projects,
            automations,
            automation_timer: 0,
            settings,
            programs,
            cli_dir,
            fatal: None,
        };
        let plugins: Vec<String> =
            core.host.plugins().iter().map(|plugin| plugin.id().to_owned()).collect();
        for plugin in plugins {
            core.post_plugin_settings(&plugin);
        }
        Ok(core)
    }

    pub(crate) async fn run(mut self, mut queue: UnboundedReceiver<Command>) -> anyhow::Result<()> {
        self.drain_queues();
        // Runs missed while the daemon was down are found by the first tick.
        self.arm_automations();
        while let Some(command) = queue.recv().await {
            if matches!(command, Command::Shutdown) {
                break;
            }
            self.handle(command);
            // Before the queue: a message waiting for the turn goes to the new session.
            self.release_idle();
            self.drain_queues();
            self.settle_waits();
            if let Some(error) = self.fatal.take() {
                return Err(error);
            }
        }
        self.shutdown();
        self.fatal.take().map_or(Ok(()), Err)
    }

    /// No provider session survives the daemon. Say so in the journal now instead of
    /// leaving it to the next start.
    fn shutdown(&mut self) {
        let ids: Vec<ThreadId> = self.supervisor.threads().map(|t| t.id.clone()).collect();
        let now = now_ms();
        for id in ids {
            let Ok(engine) = self.supervisor.engine(&id) else {
                continue;
            };
            let mut events = engine.force_settle(now);
            events.extend(engine.session_released(Some("daemon shutdown".into()), now));
            self.commit(events);
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Rpc { connection, method, params, responder } => {
                self.rpc(connection, &method, params, responder);
            }
            Command::Disconnected { connection } => {
                for subscribers in self.subscribers.values_mut() {
                    subscribers.retain(|subscriber| subscriber.connection != connection);
                }
                self.watchers.retain(|watcher| watcher.connection != connection);
                self.waiters.retain(|waiter| waiter.connection != connection);
            }
            Command::Deltas { plugin, thread, deltas } => self.deltas(&plugin, thread, deltas),
            Command::PluginCrashed { plugin, reason } => self.plugin_crashed(&plugin, &reason),
            Command::SessionStartFinished { thread, resumed, result } => {
                self.session_start_finished(thread, resumed, result);
            }
            Command::CallFinished { thread, action, result, responder } => {
                self.call_finished(thread, action, result, responder);
            }
            Command::EnvironmentPrepared { thread, prompt, result } => {
                self.environment_prepared(thread, prompt, result);
            }
            Command::EnvironmentRemoved { thread, result, responder } => {
                self.environment_removed(thread, result, responder);
            }
            Command::Threads { plugin, call, deadline, reply } => {
                let answer = if Instant::now() > deadline {
                    Err("the daemon was too busy to answer in time".to_owned())
                } else {
                    self.threads_call(&plugin, call)
                };
                let _ = reply.send(answer);
            }
            Command::ModelsListed { provider, result, responder } => {
                self.models_listed(provider, result, responder);
            }
            Command::SkillsListed { provider, result, responder } => {
                self.skills_listed(provider, result, responder);
            }
            Command::LimitsRead { only, results, responder } => {
                self.limits_read(only, results, responder);
            }
            Command::WaitTimedOut { waiter } => {
                if let Some(index) = self.waiters.iter().position(|w| w.id == waiter) {
                    let waiter = self.waiters.remove(index);
                    self.answer_wait(waiter, true);
                }
            }
            Command::AutomationTick { generation } => self.automation_tick(generation),
            Command::Shutdown => {}
        }
    }

    /// Persist events, then tell the subscribers. Persisting comes first: a client
    /// never sees an event that a restart could lose.
    fn commit(&mut self, events: Vec<ThreadEvent>) {
        // After a failed write memory and disk already disagree: write nothing more,
        // and keep the first error, which is the cause.
        if events.is_empty() || self.fatal.is_some() {
            return;
        }
        if let Err(error) = self.store.append(&events) {
            self.fatal = Some(anyhow::Error::new(error).context("cannot write the journal"));
            return;
        }
        if !self.watchers.is_empty() {
            let mut changed: Vec<&ThreadId> = Vec::new();
            for event in &events {
                let streamed = matches!(event.payload, EventPayload::ItemTextDelta { .. });
                if !streamed && !changed.contains(&&event.thread_id) {
                    changed.push(&event.thread_id);
                }
            }
            for id in changed {
                let Some(thread) = self.supervisor.thread(id) else {
                    continue;
                };
                self.watchers.retain(|watcher| {
                    let subscription = watcher.subscription;
                    let params = ThreadUpdatedNotification { subscription, thread: thread.clone() };
                    notify(&watcher.out, THREAD_UPDATED, params)
                });
            }
        }
        for event in events {
            let Some(subscribers) = self.subscribers.get_mut(&event.thread_id) else {
                continue;
            };
            subscribers.retain(|subscriber| {
                let subscription = subscriber.subscription;
                let params = ThreadEventNotification { subscription, event: event.clone() };
                notify(&subscriber.out, THREAD_EVENT, params)
            });
        }
    }
}
