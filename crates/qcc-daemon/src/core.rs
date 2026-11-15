//! The core task: the single owner of all daemon state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use base64::Engine as _;
use qcc_domain::{
    Delta, EnvironmentState, EventPayload, ModelInfo, PermissionMode, Project, ProjectId,
    ProviderId, Role, SkillRef, ThreadEnvironment, ThreadEvent, ThreadId, ThreadState,
    ThreadStatus,
};
use qcc_plugin_host::{
    Contribution, EnvironmentRequest, Outcome, PluginError, PluginHandle, PluginHost,
    PluginRequest, ProgramPaths, ProviderRequest, SessionStart, SettledThread, ThreadSummary,
    ThreadsAnswer, ThreadsCall, UiRequest,
};
use qcc_protocol::jsonrpc::{Request, Response, RpcError, codes};
use qcc_protocol::methods::{
    CommandContribution, CommandList, CommandListResult, CommandRun, CommandRunParams,
    CommandRunResult, DaemonStatus, DaemonStatusResult, Delivery, Empty, InteractionResolve,
    InteractionResolveParams, PROJECTS_UPDATED, PluginInfo, PluginList, PluginListResult,
    PluginPermissions, ProgramStatus, ProjectCreate, ProjectCreateParams, ProjectDelete,
    ProjectDeleteParams, ProjectList, ProjectListResult, ProjectResult, ProjectUpdate,
    ProjectUpdateParams, ProjectsUpdatedNotification, ProviderModels, ProviderModelsParams,
    ProviderModelsResult, SendMode, Settings, SettingsGet, SettingsResult, SettingsSet,
    SettingsSetParams, StopMode, THREAD_EVENT, THREAD_UPDATED, ThreadArchive, ThreadArchiveParams,
    ThreadChanges, ThreadChangesResult, ThreadDiff, ThreadDiffResult, ThreadEventNotification,
    ThreadEvents, ThreadEventsParams, ThreadEventsResult, ThreadGet, ThreadGetParams, ThreadList,
    ThreadListParams, ThreadListResult, ThreadRename, ThreadRenameParams, ThreadResult, ThreadSend,
    ThreadSendParams, ThreadSendResult, ThreadSetAgent, ThreadSetAgentParams, ThreadSetMode,
    ThreadSetModeParams, ThreadSetProject, ThreadSetProjectParams, ThreadSpawn, ThreadSpawnParams,
    ThreadStop, ThreadStopParams, ThreadUnqueue, ThreadUnqueueParams, ThreadUnqueueResult,
    ThreadUnsubscribe, ThreadUnsubscribeParams, ThreadUpdatedNotification, ThreadWait,
    ThreadWaitParams, ThreadWaitResult, ThreadWatch, ThreadWatchParams, UiAction, UiActionParams,
    UiAsset, UiAssetParams, UiAssetResult, UiContributions, UiContributionsResult,
    UiItemContribution, UiRender, UiRenderItem, UiRenderItemParams, UiRenderParams, UiTreeResult,
    UiViewContribution, WaitedThread,
};
use qcc_protocol::{Method, PROTOCOL_VERSION, paths};
use qcc_runtime::{NewThread, Supervisor, UuidMint};
use qcc_store::Store;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{Config, handover, plugins, projects, settings};

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
        prompt: Option<String>,
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
    /// A `thread.wait` ran out of time.
    WaitTimedOut {
        waiter: u64,
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
    sessions: HashMap<ThreadId, Session>,
    /// Threads with input waiting for their running turn to end.
    queued: HashSet<ThreadId>,
    /// Threads on their way to the archive. They take no input meanwhile.
    archiving: HashSet<ThreadId>,
    subscribers: HashMap<ThreadId, Vec<Subscriber>>,
    /// Connections following the list of threads.
    watchers: Vec<Subscriber>,
    waiters: Vec<Waiter>,
    next_waiter: u64,
    watches: Vec<Watch>,
    /// Skills plugins offer, written out where agents can read them.
    skills: Vec<SkillRef>,
    /// Oldest first.
    projects: Vec<Project>,
    settings: Settings,
    /// Where the programs plugins run live, as `settings.programs` says.
    programs: ProgramPaths,
    /// What each provider said it offers, until asked to look again.
    models: HashMap<ProviderId, Vec<ModelInfo>>,
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

fn thread_not_found(id: &ThreadId) -> RpcError {
    RpcError::new(codes::THREAD_NOT_FOUND, format!("thread {id} does not exist"))
}

impl Core {
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
        let host =
            plugins::load_all(config, &commands, &programs).context("cannot load plugins")?;
        let skills = plugins::write_skills(config, &host);
        let cli_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(PathBuf::from))
            .filter(|dir| dir.join("qcc").is_file());

        let projects = store.projects()?;
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

        Ok(Self {
            config: config.clone(),
            commands,
            store,
            supervisor,
            host,
            sessions: HashMap::new(),
            queued,
            archiving: HashSet::new(),
            subscribers: HashMap::new(),
            watchers: Vec::new(),
            waiters: Vec::new(),
            next_waiter: 0,
            watches: Vec::new(),
            skills,
            models: HashMap::new(),
            projects,
            settings,
            programs,
            cli_dir,
            fatal: None,
        })
    }

    pub(crate) async fn run(mut self, mut queue: UnboundedReceiver<Command>) -> anyhow::Result<()> {
        self.drain_queues();
        while let Some(command) = queue.recv().await {
            if matches!(command, Command::Shutdown) {
                break;
            }
            self.handle(command);
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
            Command::WaitTimedOut { waiter } => {
                if let Some(index) = self.waiters.iter().position(|w| w.id == waiter) {
                    let waiter = self.waiters.remove(index);
                    self.answer_wait(waiter, true);
                }
            }
            Command::Shutdown => {}
        }
    }

    // ----- requests -----------------------------------------------------------------

    fn rpc(&mut self, connection: u64, method: &str, raw: Value, responder: Responder) {
        match method {
            DaemonStatus::NAME => responder.reply(Ok(DaemonStatusResult {
                version: env!("CARGO_PKG_VERSION").into(),
                protocol: PROTOCOL_VERSION,
                pid: std::process::id(),
                data_dir: self.config.data_dir.display().to_string(),
                socket: self.config.socket.display().to_string(),
            })),
            ThreadSpawn::NAME => {
                let result = params::<ThreadSpawn>(raw).and_then(|p| self.thread_spawn(p));
                responder.reply(result);
            }
            ThreadSend::NAME => {
                let result = params::<ThreadSend>(raw).and_then(|p| self.thread_send(p));
                responder.reply(result);
            }
            ThreadSetMode::NAME => {
                let result = params::<ThreadSetMode>(raw).and_then(|p| self.thread_set_mode(p));
                responder.reply(result);
            }
            SettingsGet::NAME => responder.reply(Ok(self.settings_result())),
            SettingsSet::NAME => {
                let result = params::<SettingsSet>(raw).and_then(|p| self.settings_set(p));
                responder.reply(result);
            }
            ThreadRename::NAME => {
                let result = params::<ThreadRename>(raw).and_then(|p| self.thread_rename(p));
                responder.reply(result);
            }
            ThreadSetProject::NAME => {
                let result =
                    params::<ThreadSetProject>(raw).and_then(|p| self.thread_set_project(p));
                responder.reply(result);
            }
            ProjectList::NAME => {
                responder.reply(Ok(ProjectListResult { projects: self.projects.clone() }));
            }
            ProjectCreate::NAME => {
                let result = params::<ProjectCreate>(raw).and_then(|p| self.project_create(p));
                responder.reply(result);
            }
            ProjectUpdate::NAME => {
                let result = params::<ProjectUpdate>(raw).and_then(|p| self.project_update(p));
                responder.reply(result);
            }
            ProjectDelete::NAME => {
                let result = params::<ProjectDelete>(raw).and_then(|p| self.project_delete(p));
                responder.reply(result);
            }
            ThreadUnqueue::NAME => {
                let result = params::<ThreadUnqueue>(raw).and_then(|p| self.thread_unqueue(p));
                responder.reply(result);
            }
            ThreadSetAgent::NAME => {
                let result = params::<ThreadSetAgent>(raw).and_then(|p| self.thread_set_agent(p));
                responder.reply(result);
            }
            ThreadStop::NAME => match params::<ThreadStop>(raw) {
                Ok(p) => self.thread_stop(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadList::NAME => {
                let result =
                    params::<ThreadList>(raw).map(|ThreadListParams { include_archived }| {
                        let mut threads: Vec<_> = self
                            .supervisor
                            .threads()
                            .filter(|thread| include_archived || !thread.archived)
                            .cloned()
                            .collect();
                        threads.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
                        ThreadListResult { threads }
                    });
                responder.reply(result);
            }
            ThreadWatch::NAME => match params::<ThreadWatch>(raw) {
                Ok(ThreadWatchParams { subscription }) => {
                    let out = responder.out.clone();
                    let mut threads: Vec<_> = self.supervisor.threads().cloned().collect();
                    threads.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
                    responder.reply(Ok(ThreadListResult { threads }));
                    // Registered in the same step as the listing, so no change is missed.
                    self.watchers.push(Subscriber { connection, subscription, out });
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadArchive::NAME => match params::<ThreadArchive>(raw) {
                Ok(p) => self.thread_archive(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadChanges::NAME => match params::<ThreadChanges>(raw) {
                Ok(p) => self.environment_query(&p.thread_id, false, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadDiff::NAME => match params::<ThreadDiff>(raw) {
                Ok(p) => self.environment_query(&p.thread_id, true, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadGet::NAME => {
                let result = params::<ThreadGet>(raw).and_then(|ThreadGetParams { thread_id }| {
                    let thread = self.supervisor.thread(&thread_id).cloned();
                    thread.map(|thread| ThreadResult { thread }).ok_or(thread_not_found(&thread_id))
                });
                responder.reply(result);
            }
            ThreadWait::NAME => match params::<ThreadWait>(raw) {
                Ok(p) => self.thread_wait(connection, p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadEvents::NAME => match params::<ThreadEvents>(raw) {
                Ok(p) => self.thread_events(connection, p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadUnsubscribe::NAME => {
                let result = params::<ThreadUnsubscribe>(raw).map(
                    |ThreadUnsubscribeParams { subscription }| {
                        for subscribers in self.subscribers.values_mut() {
                            subscribers.retain(|s| {
                                s.connection != connection || s.subscription != subscription
                            });
                        }
                        self.watchers.retain(|watcher| {
                            watcher.connection != connection || watcher.subscription != subscription
                        });
                        Empty {}
                    },
                );
                responder.reply(result);
            }
            InteractionResolve::NAME => {
                let result =
                    params::<InteractionResolve>(raw).and_then(|p| self.interaction_resolve(p));
                responder.reply(result);
            }
            PluginList::NAME => responder.reply(Ok(self.plugin_list())),
            ProviderModels::NAME => match params::<ProviderModels>(raw) {
                Ok(p) => self.provider_models(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            CommandList::NAME => responder.reply(Ok(self.command_list())),
            CommandRun::NAME => match params::<CommandRun>(raw) {
                Ok(p) => self.command_run(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiContributions::NAME => responder.reply(Ok(self.ui_contributions())),
            UiRender::NAME => match params::<UiRender>(raw) {
                Ok(UiRenderParams { plugin, view, thread_id }) => {
                    let request = UiRequest::Render { view: view.clone(), thread: thread_id };
                    self.ui_call(&plugin, Some(&view), None, request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiAction::NAME => match params::<UiAction>(raw) {
                Ok(UiActionParams { plugin, view, thread_id, action, value }) => {
                    let request =
                        UiRequest::Action { view: view.clone(), thread: thread_id, action, value };
                    self.ui_call(&plugin, Some(&view), None, request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            UiAsset::NAME => {
                let result = params::<UiAsset>(raw).and_then(|UiAssetParams { plugin, path }| {
                    let error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
                    let handle = self
                        .host
                        .plugin(&plugin)
                        .ok_or_else(|| error(format!("no plugin {plugin:?} is loaded")))?;
                    let (data, content_type) =
                        plugins::read_asset(&self.config, handle.manifest(), &path)
                            .map_err(error)?;
                    Ok(UiAssetResult {
                        data: base64::engine::general_purpose::STANDARD.encode(data),
                        content_type: content_type.to_owned(),
                    })
                });
                responder.reply(result);
            }
            UiRenderItem::NAME => match params::<UiRenderItem>(raw) {
                Ok(UiRenderItemParams { plugin, kind, payload }) => {
                    let request =
                        UiRequest::RenderItem { kind: kind.clone(), payload: payload.to_string() };
                    self.ui_call(&plugin, None, Some(&kind), request, responder);
                }
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            unknown => responder.reply::<Empty>(Err(RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("unknown method {unknown:?}"),
            ))),
        }
    }

    fn thread_spawn(&mut self, mut p: ThreadSpawnParams) -> Result<ThreadResult, RpcError> {
        // What the user set for this agent fills in what the caller left out.
        if let Some(defaults) = self.settings.providers.get(&p.provider) {
            if p.model.is_none() {
                p.model.clone_from(&defaults.model);
            }
            if p.permission_mode.is_none() {
                p.permission_mode = defaults.permission_mode;
            }
        }
        self.check_mode(&p.provider, p.permission_mode)?;
        if let Some(parent) = &p.parent {
            if self.supervisor.thread(parent).is_none() {
                return Err(thread_not_found(parent));
            }
            if self.depth(parent) >= MAX_SPAWN_DEPTH {
                return Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    format!(
                        "thread {parent} is {MAX_SPAWN_DEPTH} levels below the thread the user \
                         started; agents may not delegate any deeper"
                    ),
                ));
            }
        }
        let cwd = std::path::Path::new(&p.cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("cwd {:?} is not an absolute path to a directory", p.cwd),
            ));
        }

        let environment = match p.environment {
            Some(id) => {
                if self.host.environment(&id).is_none() {
                    return Err(RpcError::new(
                        codes::INVALID_PARAMS,
                        format!("no plugin provides environment {id:?}"),
                    ));
                }
                Some(ThreadEnvironment {
                    id,
                    source_dir: p.cwd.clone(),
                    base: p.base,
                    state: EnvironmentState::Pending,
                })
            }
            None if p.base.is_some() => {
                return Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    "`base` means something only to an environment",
                ));
            }
            None => None,
        };

        let project = match p.project {
            Some(project) => {
                self.project(&project)?;
                Some(project)
            }
            None => {
                let inherited = p
                    .parent
                    .as_ref()
                    .and_then(|parent| self.supervisor.thread(parent)?.project.clone());
                inherited.or_else(|| {
                    qcc_domain::project_of(&self.projects, &p.cwd).map(|project| project.id.clone())
                })
            }
        };

        let id = ThreadId(format!("thr_{}", uuid::Uuid::now_v7().simple()));
        let now = now_ms();
        let mut events = self
            .supervisor
            .create(
                NewThread {
                    id: id.clone(),
                    provider: p.provider,
                    cwd: p.cwd,
                    environment,
                    model: p.model,
                    parent: p.parent,
                    project,
                    permission_mode: p.permission_mode,
                },
                now,
            )
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        if let Some(prompt) = &p.prompt
            && let Ok(engine) = self.supervisor.engine(&id)
        {
            events.extend(engine.submit_input(prompt.clone(), now));
        }
        self.commit(events);
        self.start_session(&id, p.prompt);

        let thread = self.supervisor.thread(&id).cloned().ok_or_else(|| thread_not_found(&id))?;
        Ok(ThreadResult { thread })
    }

    /// How many ancestors a thread has.
    fn depth(&self, id: &ThreadId) -> usize {
        let mut depth = 0;
        let mut current = self.supervisor.thread(id).and_then(|thread| thread.parent.clone());
        while let Some(parent) = current {
            depth += 1;
            // A cycle cannot be made through the API; the bound is only a backstop.
            if depth > MAX_SPAWN_DEPTH * 4 {
                break;
            }
            current = self.supervisor.thread(&parent).and_then(|thread| thread.parent.clone());
        }
        depth
    }

    /// The provider must exist and must be able to enforce the mode.
    fn check_mode(
        &self,
        provider: &ProviderId,
        mode: Option<PermissionMode>,
    ) -> Result<(), RpcError> {
        let Some((_, info)) = self.host.provider(provider) else {
            return Err(RpcError::new(
                codes::PROVIDER_NOT_FOUND,
                format!("no plugin provides {:?}", provider.as_str()),
            ));
        };
        match mode {
            Some(mode) if !info.capabilities.permission_modes.contains(&mode) => {
                let supported: Vec<&str> =
                    info.capabilities.permission_modes.iter().map(|m| m.as_str()).collect();
                Err(RpcError::new(
                    codes::INVALID_PARAMS,
                    format!(
                        "provider {} cannot enforce permission mode {:?}; it supports: {}",
                        provider,
                        mode.as_str(),
                        supported.join(", ")
                    ),
                ))
            }
            _ => Ok(()),
        }
    }

    fn thread_send(&mut self, p: ThreadSendParams) -> Result<ThreadSendResult, RpcError> {
        let ThreadSendParams { thread_id, text, delivery } = p;
        let provider = self.supervisor.thread(&thread_id).map(|thread| thread.provider.clone());
        let can_steer = provider
            .and_then(|provider| self.host.provider(&provider))
            .is_some_and(|(_, info)| info.capabilities.steer);
        let held = self.in_transition(&thread_id);
        self.check_open(&thread_id)?;
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let now = now_ms();

        // Nothing in the way: the message starts a turn. Queued messages go first.
        if !engine.busy() && !held && !engine.has_queued_input() {
            let events = engine.submit_input(text.clone(), now);
            self.commit(events);
            self.deliver(&thread_id, text);
            return Ok(ThreadSendResult { delivery: Delivery::Started, queue_id: None });
        }

        if delivery == SendMode::Steer
            && can_steer
            && !held
            && engine.turn_open()
            && engine.session_live()
        {
            let events = engine.steer_input(text.clone(), now);
            self.commit(events);
            self.call_provider(&thread_id, Action::Steer, None, |thread| ProviderRequest::Steer {
                thread,
                text,
            });
            return Ok(ThreadSendResult { delivery: Delivery::Steered, queue_id: None });
        }

        let events = engine.queue_input(text, now);
        let queue_id = events.iter().find_map(|event| match &event.payload {
            EventPayload::InputQueued { queue_id, .. } => Some(queue_id.clone()),
            _ => None,
        });
        self.commit(events);
        self.queued.insert(thread_id);
        Ok(ThreadSendResult { delivery: Delivery::Queued, queue_id })
    }

    fn settings_set(&mut self, p: SettingsSetParams) -> Result<SettingsResult, RpcError> {
        let SettingsSetParams { settings: next } = p;
        settings::check(&next).map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        settings::save(&self.config.settings(), &next).map_err(|error| {
            RpcError::new(codes::INTERNAL_ERROR, format!("cannot save the settings: {error}"))
        })?;
        self.programs.set(next.programs.iter().map(|(name, path)| (name.clone(), path.into())));
        self.settings = next;
        // Another program may offer other models.
        self.models.clear();
        Ok(self.settings_result())
    }

    fn settings_result(&self) -> SettingsResult {
        let mut programs: Vec<ProgramStatus> = Vec::new();
        for plugin in self.host.plugins() {
            for name in &plugin.permissions().process {
                match programs.iter_mut().find(|program| &program.name == name) {
                    Some(program) => program.plugins.push(plugin.id().to_owned()),
                    None => {
                        let (path, error) =
                            match qcc_plugin_host::resolve_program(name, &self.programs) {
                                Ok(path) => (Some(path.display().to_string()), None),
                                Err(error) => (None, Some(error)),
                            };
                        programs.push(ProgramStatus {
                            name: name.clone(),
                            plugins: vec![plugin.id().to_owned()],
                            path,
                            error,
                        });
                    }
                }
            }
        }
        programs.sort_by(|a, b| a.name.cmp(&b.name));
        SettingsResult { settings: self.settings.clone(), programs }
    }

    fn project(&self, id: &ProjectId) -> Result<&Project, RpcError> {
        self.projects.iter().find(|project| &project.id == id).ok_or_else(|| {
            RpcError::new(codes::INVALID_PARAMS, format!("project {id} does not exist"))
        })
    }

    fn thread_rename(&mut self, p: ThreadRenameParams) -> Result<ThreadResult, RpcError> {
        let title: String = p.title.trim().chars().take(200).collect();
        if title.is_empty() {
            return Err(RpcError::new(codes::INVALID_PARAMS, "a thread needs a non-empty name"));
        }
        let engine =
            self.supervisor.engine(&p.thread_id).map_err(|_| thread_not_found(&p.thread_id))?;
        let events = engine.set_title(title, now_ms());
        self.commit(events);
        let thread = self.supervisor.thread(&p.thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&p.thread_id))
    }

    fn thread_set_project(&mut self, p: ThreadSetProjectParams) -> Result<ThreadResult, RpcError> {
        let ThreadSetProjectParams { thread_id, project } = p;
        if let Some(project) = &project {
            self.project(project)?;
        }
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let events = engine.set_project(project, now_ms());
        self.commit(events);
        let thread = self.supervisor.thread(&thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&thread_id))
    }

    fn project_create(&mut self, p: ProjectCreateParams) -> Result<ProjectResult, RpcError> {
        let (name, folders) = projects::check(&p.name, &p.folders)
            .map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        let project = Project {
            id: ProjectId(format!("prj_{}", uuid::Uuid::now_v7().simple())),
            name,
            folders,
            created_at: now_ms(),
        };
        self.save_project(project.clone())?;
        Ok(ProjectResult { project })
    }

    fn project_update(&mut self, p: ProjectUpdateParams) -> Result<ProjectResult, RpcError> {
        let current = self.project(&p.id)?.clone();
        let (name, folders) = projects::check(
            p.name.as_deref().unwrap_or(&current.name),
            p.folders.as_deref().unwrap_or(&current.folders),
        )
        .map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        let project = Project { name, folders, ..current };
        self.save_project(project.clone())?;
        Ok(ProjectResult { project })
    }

    fn project_delete(&mut self, p: ProjectDeleteParams) -> Result<Empty, RpcError> {
        self.project(&p.id)?;
        self.store
            .delete_project(&p.id)
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        self.projects.retain(|project| project.id != p.id);
        // Its threads stay, outside any project.
        let members: Vec<ThreadId> = self
            .supervisor
            .threads()
            .filter(|thread| thread.project.as_ref() == Some(&p.id))
            .map(|thread| thread.id.clone())
            .collect();
        let now = now_ms();
        for id in members {
            if let Ok(engine) = self.supervisor.engine(&id) {
                let events = engine.set_project(None, now);
                self.commit(events);
            }
        }
        self.projects_changed();
        Ok(Empty {})
    }

    /// Store a new or changed project, then let threads outside any project that
    /// work in one of its folders join it.
    fn save_project(&mut self, project: Project) -> Result<(), RpcError> {
        self.store
            .save_project(&project)
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        match self.projects.iter_mut().find(|known| known.id == project.id) {
            Some(known) => *known = project.clone(),
            None => self.projects.push(project.clone()),
        }
        let joining: Vec<ThreadId> = self
            .supervisor
            .threads()
            .filter(|thread| thread.project.is_none() && !thread.archived)
            .filter(|thread| {
                let dir = thread.environment.as_ref().map_or(&thread.cwd, |env| &env.source_dir);
                qcc_domain::project_of(&self.projects, dir).map(|found| &found.id)
                    == Some(&project.id)
            })
            .map(|thread| thread.id.clone())
            .collect();
        let now = now_ms();
        for id in joining {
            if let Ok(engine) = self.supervisor.engine(&id) {
                let events = engine.set_project(Some(project.id.clone()), now);
                self.commit(events);
            }
        }
        self.projects_changed();
        Ok(())
    }

    /// Tell every connection that watches threads what the projects are now.
    fn projects_changed(&mut self) {
        let projects = self.projects.clone();
        self.watchers.retain(|watcher| {
            let notification = Request::notification(
                PROJECTS_UPDATED,
                serde_json::to_value(ProjectsUpdatedNotification {
                    subscription: watcher.subscription,
                    projects: projects.clone(),
                })
                .expect("projects serialize"),
            );
            let line = serde_json::to_string(&notification).expect("notifications serialize");
            watcher.out.send(line).is_ok()
        });
    }

    fn thread_unqueue(&mut self, p: ThreadUnqueueParams) -> Result<ThreadUnqueueResult, RpcError> {
        let ThreadUnqueueParams { thread_id, queue_id } = p;
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let Some((text, events)) = engine.unqueue(&queue_id, now_ms()) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("no message {queue_id:?} is queued; it may have been delivered already"),
            ));
        };
        if !engine.has_queued_input() {
            self.queued.remove(&thread_id);
        }
        self.commit(events);
        Ok(ThreadUnqueueResult { text })
    }

    /// The provider session is starting or stopping: input waits, and nothing else
    /// may start or stop it until the provider answers.
    fn in_transition(&self, id: &ThreadId) -> bool {
        self.sessions.get(id).is_some_and(|session| session.starting || session.stopping)
    }

    /// An archived thread takes no input, and neither does one being archived.
    fn check_open(&self, id: &ThreadId) -> Result<(), RpcError> {
        let thread = self.supervisor.thread(id).ok_or_else(|| thread_not_found(id))?;
        if thread.archived {
            return Err(RpcError::new(codes::THREAD_ARCHIVED, format!("thread {id} is archived")));
        }
        if self.archiving.contains(id) {
            return Err(RpcError::new(codes::THREAD_BUSY, "the thread is being archived"));
        }
        Ok(())
    }

    /// Hand a submitted message to the provider, attaching a session first if needed.
    fn deliver(&mut self, id: &ThreadId, text: String) {
        let live = self.supervisor.engine(id).is_ok_and(|engine| engine.session_live());
        if live {
            self.call_provider(id, Action::Input, None, |thread| ProviderRequest::Input {
                thread,
                text,
            });
        } else {
            self.start_session(id, Some(text));
        }
    }

    /// Deliver queued messages of every thread that has become free. One message per
    /// thread: the next one waits for the turn this one starts.
    fn drain_queues(&mut self) {
        if self.queued.is_empty() {
            return;
        }
        let now = now_ms();
        for id in self.queued.clone() {
            if self.in_transition(&id) {
                continue;
            }
            let Ok(engine) = self.supervisor.engine(&id) else {
                self.queued.remove(&id);
                continue;
            };
            let delivery = engine.deliver_queued(now);
            if !engine.has_queued_input() {
                self.queued.remove(&id);
            }
            if let Some((text, events)) = delivery {
                self.commit(events);
                self.deliver(&id, text);
            }
        }
    }

    fn thread_set_mode(&mut self, p: ThreadSetModeParams) -> Result<ThreadResult, RpcError> {
        let ThreadSetModeParams { thread_id, permission_mode } = p;
        let provider = self
            .supervisor
            .thread(&thread_id)
            .map(|thread| thread.provider.clone())
            .ok_or_else(|| thread_not_found(&thread_id))?;
        self.check_mode(&provider, permission_mode)?;
        self.reconfigure(&thread_id, "the permission mode", |engine, now| {
            engine.set_permission_mode(permission_mode, now)
        })
    }

    fn thread_set_agent(&mut self, p: ThreadSetAgentParams) -> Result<ThreadResult, RpcError> {
        let ThreadSetAgentParams { thread_id, provider, model } = p;
        self.check_mode(&provider, None)?;
        let supported = self
            .host
            .provider(&provider)
            .map(|(_, info)| info.capabilities.permission_modes.clone())
            .unwrap_or_default();
        let model = model.filter(|model| !model.trim().is_empty());
        self.reconfigure(&thread_id, "the agent", |engine, now| {
            let mut events = Vec::new();
            // A mode the new agent cannot enforce would fail every start.
            if engine.thread().permission_mode.is_some_and(|mode| !supported.contains(&mode)) {
                events.extend(engine.set_permission_mode(None, now));
            }
            events.extend(engine.set_agent(provider, model, now));
            events
        })
    }

    /// Change what a session is constructed with. Refused while a turn runs or the
    /// session is starting or stopping. A live session is released, so the next
    /// message starts one the new way.
    fn reconfigure(
        &mut self,
        thread_id: &ThreadId,
        what: &str,
        change: impl FnOnce(&mut qcc_runtime::ThreadEngine, u64) -> Vec<ThreadEvent>,
    ) -> Result<ThreadResult, RpcError> {
        self.check_open(thread_id)?;
        let held = self.in_transition(thread_id);
        let engine = self.supervisor.engine(thread_id).map_err(|_| thread_not_found(thread_id))?;
        if engine.busy() || held {
            return Err(RpcError::new(
                codes::THREAD_BUSY,
                format!("a turn is running; stop it before changing {what}"),
            ));
        }
        let live = engine.session_live();
        // The session belongs to the provider the thread had before the change.
        let provider = engine.thread().provider.clone();
        let events = change(engine, now_ms());
        let changed = !events.is_empty();
        self.commit(events);
        if changed && live {
            self.sessions.entry(thread_id.clone()).or_default().stopping = true;
            self.call_provider_of(thread_id, Some(provider), Action::Release, None, |thread| {
                ProviderRequest::Stop { thread }
            });
        }
        let thread = self.supervisor.thread(thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(thread_id))
    }

    fn thread_stop(&mut self, p: ThreadStopParams, responder: Responder) {
        let ThreadStopParams { thread_id, mode } = p;
        let held = self.in_transition(&thread_id);
        let Ok(engine) = self.supervisor.engine(&thread_id) else {
            return responder.reply::<Empty>(Err(thread_not_found(&thread_id)));
        };
        if held {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "the provider session is still starting or stopping; try again",
            )));
        }
        // Stop means stop: what was waiting for this turn is not run after it.
        let dropped = engine.drop_queued(now_ms());
        let engine_live = engine.session_live();
        self.commit(dropped);
        self.queued.remove(&thread_id);
        if !engine_live {
            // Nothing is attached, so nothing can be running.
            return responder.reply(Ok(Empty {}));
        }
        self.sessions.entry(thread_id.clone()).or_default().stopping = true;
        match mode {
            StopMode::Interrupt => {
                self.call_provider(&thread_id, Action::Interrupt, Some(responder), |thread| {
                    ProviderRequest::Interrupt { thread }
                });
            }
            StopMode::Release => {
                self.call_provider(&thread_id, Action::Release, Some(responder), |thread| {
                    ProviderRequest::Stop { thread }
                });
            }
        }
    }

    fn thread_events(&mut self, connection: u64, p: ThreadEventsParams, responder: Responder) {
        if self.supervisor.thread(&p.thread_id).is_none() {
            return responder.reply::<Empty>(Err(thread_not_found(&p.thread_id)));
        }
        let events = match self.store.events(&p.thread_id, p.after) {
            Ok(events) => events,
            Err(error) => {
                return responder
                    .reply::<Empty>(Err(RpcError::new(codes::INTERNAL_ERROR, error.to_string())));
            }
        };
        let out = responder.out.clone();
        responder.reply(Ok(ThreadEventsResult { events }));
        // Registered in the same step as the read: nothing can be appended in
        // between, so the backlog and the live stream meet without a gap.
        if let Some(subscription) = p.subscription {
            self.subscribers.entry(p.thread_id).or_default().push(Subscriber {
                connection,
                subscription,
                out,
            });
        }
    }

    fn interaction_resolve(&mut self, p: InteractionResolveParams) -> Result<Empty, RpcError> {
        let InteractionResolveParams { thread_id, interaction_id, resolution } = p;
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let (request_id, events) = engine
            .resolve_interaction(interaction_id, resolution.clone(), now_ms())
            .map_err(|error| RpcError::new(codes::INTERACTION_NOT_PENDING, error.to_string()))?;
        self.commit(events);
        self.call_provider(&thread_id, Action::Resolve, None, |thread| {
            ProviderRequest::ResolveInteraction { thread, request_id, resolution }
        });
        Ok(Empty {})
    }

    fn plugin_list(&self) -> PluginListResult {
        let plugins = self
            .host
            .plugins()
            .iter()
            .map(|plugin| {
                let permissions = plugin.permissions();
                let mut info = PluginInfo {
                    id: plugin.id().to_owned(),
                    name: plugin.manifest().name.clone(),
                    version: plugin.manifest().version.clone(),
                    bundled: plugins::is_bundled(plugin.id()),
                    example: plugin.manifest().example,
                    permissions: PluginPermissions {
                        deltas: permissions.deltas,
                        kv: permissions.kv,
                        timers: permissions.timers,
                        threads: permissions.threads,
                        skills: permissions.skills,
                        process: permissions.process.clone(),
                    },
                    providers: Vec::new(),
                    environments: Vec::new(),
                    commands: Vec::new(),
                    skills: Vec::new(),
                };
                for contribution in plugin.contributions() {
                    match contribution {
                        Contribution::Provider(provider) => info.providers.push(provider.clone()),
                        Contribution::Environment(environment) => {
                            info.environments.push(environment.clone());
                        }
                        Contribution::Command(command) => info.commands.push(command.clone()),
                        // Listed only if the plugin may offer it.
                        Contribution::Skill(skill) if permissions.skills => {
                            info.skills.push(skill.name.clone());
                        }
                        Contribution::Skill(_)
                        | Contribution::UiView(_)
                        | Contribution::UiItem(_) => {}
                    }
                }
                info
            })
            .collect();
        PluginListResult { plugins }
    }

    /// What every agent process sees: where it is and how to reach the daemon.
    fn provider_models(&mut self, p: ProviderModelsParams, responder: Responder) {
        let ProviderModelsParams { provider, refresh } = p;
        if !refresh && let Some(models) = self.models.get(&provider) {
            return responder.reply(Ok(ProviderModelsResult { models: models.clone() }));
        }
        let Some((plugin, _)) = self.host.provider(&provider) else {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::PROVIDER_NOT_FOUND,
                format!("no plugin provides {:?}", provider.as_str()),
            )));
        };
        let plugin = plugin.clone();
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let request = PluginRequest::Provider {
                provider: provider.clone(),
                request: ProviderRequest::ListModels,
            };
            let result = match tokio::time::timeout(MODELS_TIMEOUT, plugin.call(request)).await {
                Ok(Ok(Outcome::Models(models))) => Ok(models),
                Ok(Ok(other)) => Err(format!("the provider answered with {other:?}")),
                Ok(Err(error)) => Err(error.to_string()),
                Err(_) => Err(format!(
                    "the provider did not list its models within {} s",
                    MODELS_TIMEOUT.as_secs()
                )),
            };
            let _ = commands.send(Command::ModelsListed { provider, result, responder });
        });
    }

    fn models_listed(
        &mut self,
        provider: ProviderId,
        result: Result<Vec<ModelInfo>, String>,
        responder: Responder,
    ) {
        match result {
            Ok(models) => {
                self.models.insert(provider, models.clone());
                responder.reply(Ok(ProviderModelsResult { models }));
            }
            Err(message) => {
                responder.reply::<Empty>(Err(RpcError::new(codes::PLUGIN_ERROR, message)));
            }
        }
    }

    fn session_env(&self, id: &ThreadId) -> Vec<(String, String)> {
        // The user's variables for this agent first: those qcc sets come later and win.
        let mut env: Vec<(String, String)> = self
            .supervisor
            .thread(id)
            .and_then(|thread| self.settings.providers.get(&thread.provider))
            .map(|defaults| defaults.env.clone().into_iter().collect())
            .unwrap_or_default();
        env.extend([
            (paths::THREAD_ID_ENV.into(), id.to_string()),
            (paths::SOCKET_ENV.into(), self.config.socket.display().to_string()),
        ]);
        if let Some(dir) = &self.cli_dir {
            env.push((paths::CLI_BIN_ENV.into(), dir.join("qcc").display().to_string()));
            let path = match std::env::var("PATH") {
                Ok(path) if !path.is_empty() => format!("{}:{path}", dir.display()),
                _ => dir.display().to_string(),
            };
            env.push(("PATH".into(), path));
        }
        env
    }

    // ----- waiting --------------------------------------------------------------------

    /// Where a thread is, for someone waiting on it.
    fn thread_state(&mut self, id: &ThreadId) -> Option<ThreadState> {
        let held = self.in_transition(id);
        let archiving = self.archiving.contains(id);
        let engine = self.supervisor.engine(id).ok()?;
        let thread = engine.thread();
        Some(if thread.archived {
            ThreadState::Archived
        } else if !thread.pending_interactions.is_empty() {
            ThreadState::Waiting
        } else if held || archiving || engine.busy() || engine.has_queued_input() {
            ThreadState::Working
        } else if thread.status == ThreadStatus::Failed {
            ThreadState::Failed
        } else {
            ThreadState::Idle
        })
    }

    fn summary(&mut self, id: &ThreadId) -> Option<ThreadSummary> {
        let state = self.thread_state(id)?;
        let thread = self.supervisor.thread(id)?.clone();
        Some(ThreadSummary { thread, state })
    }

    fn last_message(&self, id: &ThreadId) -> Option<String> {
        match self.store.transcript(id, true) {
            Ok(mut entries) => entries.pop().map(|entry| entry.text),
            Err(error) => {
                tracing::error!(thread = %id, %error, "cannot read the transcript");
                None
            }
        }
    }

    fn thread_wait(&mut self, connection: u64, p: ThreadWaitParams, responder: Responder) {
        let ThreadWaitParams { thread_ids, any, timeout_ms } = p;
        if thread_ids.is_empty() {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::INVALID_PARAMS,
                "name a thread to wait on",
            )));
        }
        if let Some(missing) = thread_ids.iter().find(|id| self.supervisor.thread(id).is_none()) {
            return responder.reply::<Empty>(Err(thread_not_found(missing)));
        }
        self.next_waiter += 1;
        let id = self.next_waiter;
        self.waiters.push(Waiter { id, connection, threads: thread_ids, any, responder });
        if let Some(timeout) = timeout_ms {
            let commands = self.commands.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(timeout)).await;
                let _ = commands.send(Command::WaitTimedOut { waiter: id });
            });
        }
        // Answered right away, after this command, if nothing needs to happen first.
    }

    fn answer_wait(&mut self, waiter: Waiter, timed_out: bool) {
        let mut threads = Vec::new();
        for id in &waiter.threads {
            let (Some(state), Some(thread)) =
                (self.thread_state(id), self.supervisor.thread(id).cloned())
            else {
                continue;
            };
            let last_message = self.last_message(id);
            threads.push(WaitedThread { thread, state, last_message });
        }
        waiter.responder.reply(Ok(ThreadWaitResult { threads, timed_out }));
    }

    /// Answer the waits and fire the watches whose threads settled.
    fn settle_waits(&mut self) {
        if self.waiters.is_empty() && self.watches.is_empty() {
            return;
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let (threads, any) = (self.waiters[index].threads.clone(), self.waiters[index].any);
            let mut settled =
                threads.iter().map(|id| self.thread_state(id).is_none_or(ThreadState::is_settled));
            let done = if any { settled.any(|s| s) } else { settled.all(|s| s) };
            if done {
                let waiter = self.waiters.remove(index);
                self.answer_wait(waiter, false);
            } else {
                index += 1;
            }
        }

        let mut index = 0;
        while index < self.watches.len() {
            let thread = self.watches[index].thread.clone();
            let Some(summary) = self.summary(&thread) else {
                self.watches.remove(index);
                continue;
            };
            if !summary.state.is_settled() {
                index += 1;
                continue;
            }
            let watch = self.watches.remove(index);
            let last_message = self.last_message(&thread);
            if let Some(plugin) = self.host.plugin(&watch.plugin) {
                let settled = SettledThread { summary, last_message };
                if plugin.post(PluginRequest::ThreadSettled(Box::new(settled))).is_err() {
                    tracing::warn!(plugin = watch.plugin, "a watching plugin is gone");
                }
            }
        }
    }

    // ----- the `threads` import -------------------------------------------------------

    fn threads_call(&mut self, plugin: &str, call: ThreadsCall) -> Result<ThreadsAnswer, String> {
        let summary = |core: &mut Self, id: &ThreadId| {
            core.summary(id)
                .map(|summary| ThreadsAnswer::Thread(Box::new(summary)))
                .ok_or_else(|| format!("thread {id} does not exist"))
        };
        match call {
            ThreadsCall::Spawn(spawn) => {
                tracing::info!(plugin, provider = %spawn.provider, "a plugin spawns a thread");
                let result = self
                    .thread_spawn(ThreadSpawnParams {
                        provider: spawn.provider,
                        cwd: spawn.cwd,
                        environment: spawn.environment,
                        base: spawn.base,
                        prompt: spawn.prompt,
                        model: spawn.model,
                        parent: spawn.parent,
                        project: None,
                        permission_mode: spawn.permission_mode,
                    })
                    .map_err(|error| error.message)?;
                summary(self, &result.thread.id)
            }
            ThreadsCall::Send { thread, text } => self
                .thread_send(ThreadSendParams {
                    thread_id: thread,
                    text,
                    delivery: SendMode::Queue,
                })
                .map(|_| ThreadsAnswer::Done)
                .map_err(|error| error.message),
            ThreadsCall::Get(thread) => summary(self, &thread),
            ThreadsCall::Transcript(thread) => {
                if self.supervisor.thread(&thread).is_none() {
                    return Err(format!("thread {thread} does not exist"));
                }
                self.store
                    .transcript(&thread, false)
                    .map(ThreadsAnswer::Transcript)
                    .map_err(|error| error.to_string())
            }
            ThreadsCall::Watch(thread) => {
                if self.supervisor.thread(&thread).is_none() {
                    return Err(format!("thread {thread} does not exist"));
                }
                self.watches.push(Watch { plugin: plugin.to_owned(), thread });
                Ok(ThreadsAnswer::Done)
            }
        }
    }

    // ----- plugin commands ------------------------------------------------------------

    fn command_list(&self) -> CommandListResult {
        let mut commands = Vec::new();
        for plugin in self.host.plugins() {
            for contribution in plugin.contributions() {
                if let Contribution::Command(command) = contribution {
                    commands.push(CommandContribution {
                        plugin: plugin.id().to_owned(),
                        command: command.clone(),
                    });
                }
            }
        }
        CommandListResult { commands }
    }

    fn command_run(&mut self, p: CommandRunParams, responder: Responder) {
        let CommandRunParams { plugin: plugin_id, command, thread_id, args } = p;
        let invalid = |message: String| RpcError::new(codes::INVALID_PARAMS, message);
        let Some(plugin) = self.host.plugin(&plugin_id).cloned() else {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::PLUGIN_ERROR,
                format!("no plugin {plugin_id:?} is loaded"),
            )));
        };
        let info = plugin.contributions().iter().find_map(|contribution| match contribution {
            Contribution::Command(info) if info.id == command => Some(info.clone()),
            _ => None,
        });
        let Some(info) = info else {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::PLUGIN_ERROR,
                format!("plugin {plugin_id:?} has no command {command:?}"),
            )));
        };
        let thread = match (info.thread, thread_id) {
            (true, None) => {
                return responder
                    .reply::<Empty>(Err(invalid(format!("{command} acts on a thread; name one"))));
            }
            (true, Some(id)) if self.supervisor.thread(&id).is_none() => {
                return responder.reply::<Empty>(Err(thread_not_found(&id)));
            }
            (true, thread) => thread,
            (false, _) => None,
        };
        if let Some(unknown) =
            args.keys().find(|name| !info.params.iter().any(|p| &p.name == *name))
        {
            return responder
                .reply::<Empty>(Err(invalid(format!("{command} takes no parameter {unknown:?}"))));
        }
        if let Some(missing) = info
            .params
            .iter()
            .find(|p| p.required && args.get(&p.name).is_none_or(|value| value.trim().is_empty()))
        {
            return responder.reply::<Empty>(Err(invalid(format!(
                "{command} needs {:?} ({})",
                missing.name, missing.label
            ))));
        }
        let request = PluginRequest::Command { command, thread, args: args.into_iter().collect() };
        tokio::spawn(async move {
            let answer = tokio::time::timeout(QUERY_TIMEOUT, plugin.call(request)).await;
            let error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
            let result = match answer {
                Ok(Ok(Outcome::CommandResult { message, thread })) => {
                    Ok(CommandRunResult { message, thread_id: thread })
                }
                Ok(Ok(other)) => Err(error(format!("the plugin answered with {other:?}"))),
                Ok(Err(failure)) => Err(error(failure.to_string())),
                Err(_) => Err(error("the plugin did not answer in time".into())),
            };
            responder.reply(result);
        });
    }

    // ----- plugin UI ------------------------------------------------------------------

    fn ui_contributions(&self) -> UiContributionsResult {
        let mut result = UiContributionsResult { views: Vec::new(), items: Vec::new() };
        for plugin in self.host.plugins() {
            let providers: Vec<ProviderId> = plugin
                .contributions()
                .iter()
                .filter_map(|contribution| match contribution {
                    Contribution::Provider(provider) => Some(provider.id.clone()),
                    _ => None,
                })
                .collect();
            for contribution in plugin.contributions() {
                match contribution {
                    Contribution::UiView(view) => result.views.push(UiViewContribution {
                        plugin: plugin.id().to_owned(),
                        view: view.clone(),
                        providers: providers.clone(),
                    }),
                    // A plugin draws only its own kinds: it cannot take over the
                    // items of another plugin.
                    Contribution::UiItem(kind) if owns_kind(plugin.id(), kind) => {
                        result.items.push(UiItemContribution {
                            plugin: plugin.id().to_owned(),
                            kind: kind.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }
        result
    }

    /// Pass a drawing request to the plugin that contributes the view or item kind.
    fn ui_call(
        &mut self,
        plugin_id: &str,
        view: Option<&str>,
        kind: Option<&str>,
        request: UiRequest,
        responder: Responder,
    ) {
        let plugin_error = |message: String| RpcError::new(codes::PLUGIN_ERROR, message);
        let Some(plugin) = self.host.plugins().iter().find(|plugin| plugin.id() == plugin_id)
        else {
            return responder
                .reply::<Empty>(Err(plugin_error(format!("no plugin {plugin_id:?} is loaded"))));
        };
        let contributed = plugin.contributions().iter().any(|contribution| match contribution {
            Contribution::UiView(info) => view == Some(info.id.as_str()),
            Contribution::UiItem(own) => kind == Some(own.as_str()) && owns_kind(plugin_id, own),
            _ => false,
        });
        if !contributed {
            let what = view.or(kind).unwrap_or_default();
            return responder.reply::<Empty>(Err(plugin_error(format!(
                "plugin {plugin_id:?} does not contribute {what:?}"
            ))));
        }
        let plugin = plugin.clone();
        tokio::spawn(async move {
            let answer =
                tokio::time::timeout(QUERY_TIMEOUT, plugin.call(PluginRequest::Ui(request))).await;
            let result = match answer {
                Ok(Ok(Outcome::UiTree(tree))) => {
                    serde_json::from_str::<Value>(&tree).map(|tree| UiTreeResult { tree }).map_err(
                        |error| plugin_error(format!("the plugin drew invalid JSON: {error}")),
                    )
                }
                Ok(Ok(other)) => Err(plugin_error(format!("the plugin answered with {other:?}"))),
                Ok(Err(error)) => Err(plugin_error(error.to_string())),
                Err(_) => Err(plugin_error("the plugin did not answer in time".into())),
            };
            responder.reply(result);
        });
    }

    // ----- provider sessions ----------------------------------------------------------

    /// Ask the thread's provider to start a session, resuming the previous one when
    /// the provider can. The answer comes back as [`Command::SessionStartFinished`].
    fn start_session(&mut self, id: &ThreadId, prompt: Option<String>) {
        let Some(thread) = self.supervisor.thread(id).cloned() else {
            return;
        };
        // The agent must never run in the source directory of a thread that asked
        // for a directory of its own.
        if let Some(environment) = thread.environment.clone()
            && environment.state != EnvironmentState::Ready
        {
            self.prepare_environment(id, environment, prompt);
            return;
        }
        let Some((plugin, info)) = self.host.provider(&thread.provider) else {
            let message = format!("no plugin provides {:?}", thread.provider.as_str());
            self.session_failed(id, message);
            return;
        };
        let extra_dirs = self.extra_dirs(&thread);
        let resume = thread.provider_session.filter(|_| info.capabilities.resume);
        let resumed = resume.is_some();
        let prompt = match prompt {
            Some(message) if thread.handover => Some(self.handover_prompt(id, message)),
            prompt => prompt,
        };
        let request = PluginRequest::Provider {
            provider: thread.provider,
            request: ProviderRequest::Start(SessionStart {
                thread: id.clone(),
                cwd: thread.cwd,
                model: thread.model,
                resume,
                prompt,
                permission_mode: thread.permission_mode,
                env: self.session_env(id),
                skills: self.skills.clone(),
                extra_dirs,
            }),
        };
        self.sessions.entry(id.clone()).or_default().starting = true;
        let (plugin, commands, thread) = (plugin.clone(), self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = plugin.call(request).await;
            let _ = commands.send(Command::SessionStartFinished { thread, resumed, result });
        });
    }

    /// The folders of the thread's project other than the one it works in. A thread
    /// in a worktree works in a copy of its source folder, which is left out too.
    fn extra_dirs(&self, thread: &qcc_domain::Thread) -> Vec<String> {
        let Some(project) =
            self.projects.iter().find(|project| Some(&project.id) == thread.project.as_ref())
        else {
            return Vec::new();
        };
        let own: Vec<&str> = std::iter::once(thread.cwd.as_str())
            .chain(thread.environment.as_ref().map(|env| env.source_dir.as_str()))
            .collect();
        project
            .folders
            .iter()
            .filter(|folder| {
                let folder = std::path::Path::new(folder);
                // Neither the folder it works in nor one inside it, nor one around it.
                !own.iter().any(|dir| {
                    let dir = std::path::Path::new(dir);
                    dir.starts_with(folder) || folder.starts_with(dir)
                })
            })
            .cloned()
            .collect()
    }

    /// The first message for an agent that takes over from another one.
    fn handover_prompt(&self, id: &ThreadId, message: String) -> String {
        let events = self.store.events(id, 0).unwrap_or_default();
        // The agent that had the thread before the latest change.
        let mut providers = events.iter().filter_map(|event| match &event.payload {
            EventPayload::ThreadCreated { thread } => Some(thread.provider.clone()),
            EventPayload::AgentChanged { provider, .. } => Some(provider.clone()),
            _ => None,
        });
        let mut previous = None;
        let mut current = providers.next();
        for provider in providers {
            if Some(&provider) != current.as_ref() {
                previous = current.replace(provider);
            }
        }
        let previous = previous.map_or_else(
            || "another agent".to_owned(),
            |provider| {
                self.host
                    .provider(&provider)
                    .map_or(provider.to_string(), |(_, info)| info.name.clone())
            },
        );
        let mut history = self.store.transcript(id, false).unwrap_or_default();
        // The message itself was journaled before the session was asked to start.
        if history.last().is_some_and(|last| last.role == Role::User && last.text == message) {
            history.pop();
        }
        handover::prompt(&previous, &history, &message)
    }

    fn session_start_finished(
        &mut self,
        id: ThreadId,
        resumed: bool,
        result: Result<Outcome, PluginError>,
    ) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if !session.starting {
            // A crash was reported while the start was in flight and already settled it.
            return;
        }
        let session = std::mem::take(session);
        match result {
            Ok(Outcome::SessionStarted { provider_session }) => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.session_started(provider_session, resumed, now_ms());
                    self.commit(events);
                }
                self.apply_deltas(&id, session.buffered);
            }
            Ok(other) => {
                self.apply_deltas(&id, session.buffered);
                self.session_failed(&id, format!("provider answered start with {other:?}"));
            }
            Err(error) => {
                self.apply_deltas(&id, session.buffered);
                self.session_failed(&id, error.to_string());
            }
        }
    }

    // ----- environments ---------------------------------------------------------------

    /// Ask the thread's environment for its working directory. The session starts
    /// when the answer comes back as [`Command::EnvironmentPrepared`].
    fn prepare_environment(
        &mut self,
        id: &ThreadId,
        environment: ThreadEnvironment,
        prompt: Option<String>,
    ) {
        if environment.state == EnvironmentState::Removed {
            self.session_failed(id, "the thread's environment was removed".into());
            return;
        }
        if environment.state == EnvironmentState::Failed
            && let Ok(engine) = self.supervisor.engine(id)
        {
            let events = engine.record(EventPayload::EnvironmentPreparing, now_ms());
            self.commit(events);
        }
        let Some(plugin) = self.host.environment(&environment.id).map(|(plugin, _)| plugin.clone())
        else {
            let message = format!("no plugin provides environment {:?}", environment.id);
            self.environment_failed(id, message);
            return;
        };
        let root = self.config.environment_root(&environment.id);
        if let Err(error) = std::fs::create_dir_all(&root) {
            self.environment_failed(id, format!("cannot create {}: {error}", root.display()));
            return;
        }
        let request = EnvironmentRequest::Prepare {
            thread: id.clone(),
            source_dir: environment.source_dir,
            root: root.display().to_string(),
            base: environment.base,
        };
        // Until the directory exists the thread is as busy as one whose provider is
        // starting: input queues behind it.
        self.sessions.entry(id.clone()).or_default().starting = true;
        let (commands, thread) = (self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = call_environment(plugin, environment.id, request, PREPARE_TIMEOUT).await;
            let _ = commands.send(Command::EnvironmentPrepared { thread, prompt, result });
        });
    }

    fn environment_prepared(
        &mut self,
        id: ThreadId,
        prompt: Option<String>,
        result: Result<Outcome, PluginError>,
    ) {
        match self.sessions.get_mut(&id) {
            Some(session) if session.starting => session.starting = false,
            _ => return,
        }
        match result {
            Ok(Outcome::EnvironmentReady { cwd }) => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.record(EventPayload::EnvironmentReady { cwd }, now_ms());
                    self.commit(events);
                }
                self.start_session(&id, prompt);
            }
            Ok(other) => {
                self.environment_failed(
                    &id,
                    format!("environment answered prepare with {other:?}"),
                );
            }
            Err(error) => self.environment_failed(&id, error.to_string()),
        }
    }

    fn environment_failed(&mut self, id: &ThreadId, message: String) {
        if let Ok(engine) = self.supervisor.engine(id) {
            let failed = EventPayload::EnvironmentFailed { message: message.clone() };
            let events = engine.record(failed, now_ms());
            self.commit(events);
        }
        self.session_failed(id, format!("cannot prepare the working directory: {message}"));
    }

    /// The environment of a thread that has one ready, and the plugin behind it.
    fn ready_environment(
        &self,
        id: &ThreadId,
    ) -> Result<Option<(PluginHandle, String, String)>, RpcError> {
        let thread = self.supervisor.thread(id).ok_or_else(|| thread_not_found(id))?;
        let Some(environment) =
            thread.environment.as_ref().filter(|e| e.state == EnvironmentState::Ready)
        else {
            return Ok(None);
        };
        let (plugin, _) = self.host.environment(&environment.id).ok_or_else(|| {
            environment_error(format!("no plugin provides environment {:?}", environment.id))
        })?;
        Ok(Some((plugin.clone(), environment.id.clone(), thread.cwd.clone())))
    }

    /// `thread.changes` and `thread.diff`: answered by the environment plugin, off
    /// the core task.
    fn environment_query(&mut self, id: &ThreadId, diff: bool, responder: Responder) {
        let (plugin, environment, cwd) = match self.ready_environment(id) {
            Ok(Some(found)) => found,
            Ok(None) => {
                return responder.reply::<Empty>(Err(environment_error(format!(
                    "thread {id} has no environment to track its changes; \
                     start threads with an environment for that"
                ))));
            }
            Err(error) => return responder.reply::<Empty>(Err(error)),
        };
        let thread = id.clone();
        let request = if diff {
            EnvironmentRequest::Diff { thread, cwd }
        } else {
            EnvironmentRequest::Changes { thread, cwd }
        };
        tokio::spawn(async move {
            match call_environment(plugin, environment, request, QUERY_TIMEOUT).await {
                Ok(Outcome::EnvironmentChanges(changes)) => {
                    responder.reply(Ok(ThreadChangesResult { changes }));
                }
                Ok(Outcome::EnvironmentDiff { diff, truncated }) => {
                    responder.reply(Ok(ThreadDiffResult { diff, truncated }));
                }
                Ok(other) => responder.reply::<Empty>(Err(environment_error(format!(
                    "environment answered with {other:?}"
                )))),
                Err(error) => responder.reply::<Empty>(Err(environment_error(error.to_string()))),
            }
        });
    }

    // ----- archiving ------------------------------------------------------------------

    /// Release the session, then remove the environment, then mark the thread.
    fn thread_archive(&mut self, p: ThreadArchiveParams, responder: Responder) {
        let ThreadArchiveParams { thread_id, force } = p;
        let Some(thread) = self.supervisor.thread(&thread_id).cloned() else {
            return responder.reply::<Empty>(Err(thread_not_found(&thread_id)));
        };
        if thread.archived {
            return responder.reply(Ok(ThreadResult { thread }));
        }
        let held = self.in_transition(&thread_id);
        let Ok(engine) = self.supervisor.engine(&thread_id) else {
            return responder.reply::<Empty>(Err(thread_not_found(&thread_id)));
        };
        if engine.busy() || held || self.archiving.contains(&thread_id) {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "the thread is working; stop it before archiving it",
            )));
        }
        let dropped = engine.drop_queued(now_ms());
        let live = engine.session_live();
        self.commit(dropped);
        self.queued.remove(&thread_id);
        self.archiving.insert(thread_id.clone());
        if live {
            self.call_provider(&thread_id, Action::Archive { force }, Some(responder), |thread| {
                ProviderRequest::Stop { thread }
            });
        } else {
            self.remove_environment(thread_id, force, responder);
        }
    }

    fn remove_environment(&mut self, id: ThreadId, force: bool, responder: Responder) {
        let (plugin, environment, cwd) = match self.ready_environment(&id) {
            Ok(Some(found)) => found,
            // Nothing was made, so there is nothing to remove.
            Ok(None) => return self.finish_archive(&id, false, responder),
            // The plugin that made the directory is gone; forcing leaves it behind.
            Err(_) if force => return self.finish_archive(&id, false, responder),
            Err(error) => {
                self.archiving.remove(&id);
                return responder.reply::<Empty>(Err(error));
            }
        };
        let request = EnvironmentRequest::Remove { thread: id.clone(), cwd, force };
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let result = call_environment(plugin, environment, request, REMOVE_TIMEOUT).await;
            let _ = commands.send(Command::EnvironmentRemoved { thread: id, result, responder });
        });
    }

    fn environment_removed(
        &mut self,
        id: ThreadId,
        result: Result<Outcome, PluginError>,
        responder: Responder,
    ) {
        match result {
            Ok(_) => self.finish_archive(&id, true, responder),
            Err(error) => {
                // The thread stays as it was, minus its session.
                self.archiving.remove(&id);
                responder.reply::<Empty>(Err(environment_error(error.to_string())));
            }
        }
    }

    fn finish_archive(&mut self, id: &ThreadId, removed: bool, responder: Responder) {
        self.archiving.remove(id);
        self.sessions.remove(id);
        let Ok(engine) = self.supervisor.engine(id) else {
            return responder.reply::<Empty>(Err(thread_not_found(id)));
        };
        let now = now_ms();
        let mut events = Vec::new();
        if removed {
            events.extend(engine.record(EventPayload::EnvironmentRemoved, now));
        }
        events.extend(engine.record(EventPayload::ThreadArchived, now));
        let thread = engine.thread().clone();
        self.commit(events);
        responder.reply(Ok(ThreadResult { thread }));
    }

    /// Run a provider request for a thread off the core task. The answer comes back
    /// as [`Command::CallFinished`].
    fn call_provider(
        &mut self,
        id: &ThreadId,
        action: Action,
        responder: Option<Responder>,
        request: impl FnOnce(ThreadId) -> ProviderRequest,
    ) {
        self.call_provider_of(id, None, action, responder, request);
    }

    /// Like [`Core::call_provider`], to `provider` instead of the thread's own.
    fn call_provider_of(
        &mut self,
        id: &ThreadId,
        provider: Option<ProviderId>,
        action: Action,
        responder: Option<Responder>,
        request: impl FnOnce(ThreadId) -> ProviderRequest,
    ) {
        let provider =
            provider.or_else(|| self.supervisor.thread(id).map(|thread| thread.provider.clone()));
        let plugin = provider.as_ref().and_then(|p| self.host.provider(p)).map(|(p, _)| p.clone());
        let (Some(provider), Some(plugin)) = (provider, plugin) else {
            let _ = self.commands.send(Command::CallFinished {
                thread: id.clone(),
                action,
                result: Err(PluginError::Gone),
                responder,
            });
            return;
        };
        let request = PluginRequest::Provider { provider, request: request(id.clone()) };
        let (commands, thread) = (self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = plugin.call(request).await;
            let _ = commands.send(Command::CallFinished { thread, action, result, responder });
        });
    }

    fn call_finished(
        &mut self,
        id: ThreadId,
        action: Action,
        result: Result<Outcome, PluginError>,
        responder: Option<Responder>,
    ) {
        match &result {
            Ok(_) => {}
            // The provider refused; it is still alive and the user should see why.
            Err(PluginError::Rejected(message)) => {
                if matches!(action, Action::Input)
                    && let Ok(engine) = self.supervisor.engine(&id)
                {
                    // No turn will open for this message; do not wait for one.
                    engine.input_rejected();
                }
                self.apply_deltas(&id, vec![Delta::ProviderError { message: message.clone() }]);
            }
            // Reported separately through `PluginCrashed`, which settles every thread
            // of the plugin.
            Err(PluginError::Crashed(_)) => {}
            Err(PluginError::Gone) => self.session_failed(&id, "provider plugin is gone".into()),
        }
        let now = now_ms();
        if matches!(action, Action::Interrupt | Action::Release)
            && let Some(session) = self.sessions.get_mut(&id)
        {
            session.stopping = false;
        }
        match action {
            Action::Input | Action::Steer | Action::Resolve => {}
            Action::Interrupt => {
                // The backstop: a bridge that settled its own turn makes this a no-op.
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.force_settle(now);
                    self.commit(events);
                }
            }
            Action::Release | Action::Archive { .. } => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let mut events = engine.force_settle(now);
                    events.extend(engine.session_released(Some("released".into()), now));
                    self.commit(events);
                }
            }
        }
        if let Action::Archive { force } = action {
            if let Some(responder) = responder {
                self.remove_environment(id, force, responder);
            }
            return;
        }
        if let Some(responder) = responder {
            // Whatever the provider said, the thread is settled now, which is what
            // the caller asked for.
            responder.reply(Ok(Empty {}));
        }
    }

    fn deltas(&mut self, plugin: &str, id: ThreadId, deltas: Vec<Delta>) {
        let owner = self
            .supervisor
            .thread(&id)
            .and_then(|thread| self.host.provider(&thread.provider))
            .map(|(owner, _)| owner.id().to_owned());
        if owner.as_deref() != Some(plugin) {
            // A plugin may only speak for threads of its own providers.
            tracing::warn!(plugin, thread = %id, "dropped deltas for a thread the plugin does not own");
            return;
        }
        match self.sessions.get_mut(&id) {
            Some(session) if session.starting => session.buffered.extend(deltas),
            _ => self.apply_deltas(&id, deltas),
        }
    }

    fn apply_deltas(&mut self, id: &ThreadId, deltas: Vec<Delta>) {
        if deltas.is_empty() {
            return;
        }
        let Ok(engine) = self.supervisor.engine(id) else {
            return;
        };
        let events = engine.apply_deltas(deltas, now_ms());
        for violation in engine.take_violations() {
            tracing::warn!(thread = %id, %violation, "bridge broke the delta grammar");
        }
        self.commit(events);
    }

    fn session_failed(&mut self, id: &ThreadId, message: String) {
        self.sessions.remove(id);
        self.queued.remove(id);
        if let Ok(engine) = self.supervisor.engine(id) {
            let now = now_ms();
            let mut events = engine.session_failed(message, now);
            // Retrying the queue against a provider that just failed would loop.
            events.extend(engine.drop_queued(now));
            self.commit(events);
        }
    }

    fn plugin_crashed(&mut self, plugin: &str, reason: &str) {
        let affected: Vec<ThreadId> = self
            .supervisor
            .threads()
            .filter(|thread| {
                self.host.provider(&thread.provider).is_some_and(|(owner, _)| owner.id() == plugin)
            })
            .map(|thread| thread.id.clone())
            .collect();
        for id in affected {
            let starting = self.sessions.get(&id).is_some_and(|session| session.starting);
            let live = self.supervisor.engine(&id).is_ok_and(|engine| engine.session_live());
            if starting || live {
                self.session_failed(&id, format!("plugin {plugin} crashed: {reason}"));
            }
        }
    }

    // ----- journal --------------------------------------------------------------------

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
                    let notification = Request::notification(
                        THREAD_UPDATED,
                        serde_json::to_value(ThreadUpdatedNotification {
                            subscription: watcher.subscription,
                            thread: thread.clone(),
                        })
                        .expect("threads serialize"),
                    );
                    let line =
                        serde_json::to_string(&notification).expect("notifications serialize");
                    watcher.out.send(line).is_ok()
                });
            }
        }
        for event in events {
            let Some(subscribers) = self.subscribers.get_mut(&event.thread_id) else {
                continue;
            };
            subscribers.retain(|subscriber| {
                let notification = Request::notification(
                    THREAD_EVENT,
                    serde_json::to_value(ThreadEventNotification {
                        subscription: subscriber.subscription,
                        event: event.clone(),
                    })
                    .expect("events serialize"),
                );
                let line = serde_json::to_string(&notification).expect("notifications serialize");
                subscriber.out.send(line).is_ok()
            });
        }
    }
}
