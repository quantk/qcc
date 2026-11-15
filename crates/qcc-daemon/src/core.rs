//! The core task: the single owner of all daemon state.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use qcc_domain::{
    Delta, EventPayload, PermissionMode, ProviderId, ThreadEvent, ThreadId, ThreadStatus,
};
use qcc_plugin_host::{
    Contribution, Outcome, PluginError, PluginHost, PluginRequest, ProviderRequest, SessionStart,
};
use qcc_protocol::jsonrpc::{Request, Response, RpcError, codes};
use qcc_protocol::methods::{
    DaemonStatus, DaemonStatusResult, Delivery, Empty, InteractionResolve,
    InteractionResolveParams, PluginInfo, PluginList, PluginListResult, PluginPermissions,
    SendMode, StopMode, THREAD_EVENT, ThreadEventNotification, ThreadEvents, ThreadEventsParams,
    ThreadEventsResult, ThreadGet, ThreadGetParams, ThreadList, ThreadListResult, ThreadResult,
    ThreadSend, ThreadSendParams, ThreadSendResult, ThreadSetMode, ThreadSetModeParams,
    ThreadSpawn, ThreadSpawnParams, ThreadStop, ThreadStopParams, ThreadUnsubscribe,
    ThreadUnsubscribeParams,
};
use qcc_protocol::{Method, PROTOCOL_VERSION, paths};
use qcc_runtime::{NewThread, Supervisor, UuidMint};
use qcc_store::Store;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::{Config, plugins};

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
    Shutdown,
}

/// Per-thread state that does not outlive the daemon process.
#[derive(Default)]
struct Session {
    /// The provider was asked to start and has not answered yet.
    starting: bool,
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
    subscribers: HashMap<ThreadId, Vec<Subscriber>>,
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
        let host = plugins::load_all(config, &commands).context("cannot load plugins")?;

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
            subscribers: HashMap::new(),
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
            }
            Command::Deltas { plugin, thread, deltas } => self.deltas(&plugin, thread, deltas),
            Command::PluginCrashed { plugin, reason } => self.plugin_crashed(&plugin, &reason),
            Command::SessionStartFinished { thread, resumed, result } => {
                self.session_start_finished(thread, resumed, result);
            }
            Command::CallFinished { thread, action, result, responder } => {
                self.call_finished(thread, action, result, responder);
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
            ThreadStop::NAME => match params::<ThreadStop>(raw) {
                Ok(p) => self.thread_stop(p, responder),
                Err(error) => responder.reply::<Empty>(Err(error)),
            },
            ThreadList::NAME => {
                let mut threads: Vec<_> = self.supervisor.threads().cloned().collect();
                threads.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
                responder.reply(Ok(ThreadListResult { threads }));
            }
            ThreadGet::NAME => {
                let result = params::<ThreadGet>(raw).and_then(|ThreadGetParams { thread_id }| {
                    let thread = self.supervisor.thread(&thread_id).cloned();
                    thread.map(|thread| ThreadResult { thread }).ok_or(thread_not_found(&thread_id))
                });
                responder.reply(result);
            }
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
            unknown => responder.reply::<Empty>(Err(RpcError::new(
                codes::METHOD_NOT_FOUND,
                format!("unknown method {unknown:?}"),
            ))),
        }
    }

    fn thread_spawn(&mut self, p: ThreadSpawnParams) -> Result<ThreadResult, RpcError> {
        self.check_mode(&p.provider, p.permission_mode)?;
        if let Some(parent) = &p.parent
            && self.supervisor.thread(parent).is_none()
        {
            return Err(thread_not_found(parent));
        }
        let cwd = std::path::Path::new(&p.cwd);
        if !cwd.is_absolute() || !cwd.is_dir() {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("cwd {:?} is not an absolute path to a directory", p.cwd),
            ));
        }

        let id = ThreadId(format!("thr_{}", uuid::Uuid::now_v7().simple()));
        let now = now_ms();
        let mut events = self
            .supervisor
            .create(
                NewThread {
                    id: id.clone(),
                    provider: p.provider,
                    cwd: p.cwd,
                    model: p.model,
                    parent: p.parent,
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
        let starting = self.sessions.get(&thread_id).is_some_and(|session| session.starting);
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let now = now_ms();

        // Nothing in the way: the message starts a turn. Queued messages go first.
        if !engine.busy() && !starting && !engine.has_queued_input() {
            let events = engine.submit_input(text.clone(), now);
            self.commit(events);
            self.deliver(&thread_id, text);
            return Ok(ThreadSendResult { delivery: Delivery::Started, queue_id: None });
        }

        if delivery == SendMode::Steer && can_steer && engine.turn_open() && engine.session_live() {
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
            if self.sessions.get(&id).is_some_and(|session| session.starting) {
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
        let starting = self.sessions.get(&thread_id).is_some_and(|session| session.starting);
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        if engine.busy() || starting {
            return Err(RpcError::new(
                codes::THREAD_BUSY,
                "a turn is running; stop it before changing the permission mode",
            ));
        }
        let live = engine.session_live();
        let events = engine.set_permission_mode(permission_mode, now_ms());
        let changed = !events.is_empty();
        self.commit(events);
        if changed && live {
            // A mode is fixed when a session is constructed. Release this one; the
            // next message resumes the conversation in the new mode.
            self.call_provider(&thread_id, Action::Release, None, |thread| ProviderRequest::Stop {
                thread,
            });
        }
        let thread = self.supervisor.thread(&thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&thread_id))
    }

    fn thread_stop(&mut self, p: ThreadStopParams, responder: Responder) {
        let ThreadStopParams { thread_id, mode } = p;
        let Ok(engine) = self.supervisor.engine(&thread_id) else {
            return responder.reply::<Empty>(Err(thread_not_found(&thread_id)));
        };
        if self.sessions.get(&thread_id).is_some_and(|session| session.starting) {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "the provider session is still starting; try again",
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
                    permissions: PluginPermissions {
                        deltas: permissions.deltas,
                        kv: permissions.kv,
                        timers: permissions.timers,
                        process: permissions.process.clone(),
                    },
                    providers: Vec::new(),
                    environments: Vec::new(),
                };
                for contribution in plugin.contributions() {
                    match contribution {
                        Contribution::Provider(provider) => info.providers.push(provider.clone()),
                        Contribution::Environment(environment) => {
                            info.environments.push(environment.clone());
                        }
                    }
                }
                info
            })
            .collect();
        PluginListResult { plugins }
    }

    // ----- provider sessions ----------------------------------------------------------

    /// Ask the thread's provider to start a session, resuming the previous one when
    /// the provider can. The answer comes back as [`Command::SessionStartFinished`].
    fn start_session(&mut self, id: &ThreadId, prompt: Option<String>) {
        let Some(thread) = self.supervisor.thread(id).cloned() else {
            return;
        };
        let Some((plugin, info)) = self.host.provider(&thread.provider) else {
            let message = format!("no plugin provides {:?}", thread.provider.as_str());
            self.session_failed(id, message);
            return;
        };
        let resume = thread.provider_session.filter(|_| info.capabilities.resume);
        let resumed = resume.is_some();
        let request = PluginRequest::Provider {
            provider: thread.provider,
            request: ProviderRequest::Start(SessionStart {
                thread: id.clone(),
                cwd: thread.cwd,
                model: thread.model,
                resume,
                prompt,
                permission_mode: thread.permission_mode,
                env: vec![
                    (paths::THREAD_ID_ENV.into(), id.to_string()),
                    (paths::SOCKET_ENV.into(), self.config.socket.display().to_string()),
                ],
            }),
        };
        self.sessions.entry(id.clone()).or_default().starting = true;
        let (plugin, commands, thread) = (plugin.clone(), self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = plugin.call(request).await;
            let _ = commands.send(Command::SessionStartFinished { thread, resumed, result });
        });
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

    /// Run a provider request for a thread off the core task. The answer comes back
    /// as [`Command::CallFinished`].
    fn call_provider(
        &mut self,
        id: &ThreadId,
        action: Action,
        responder: Option<Responder>,
        request: impl FnOnce(ThreadId) -> ProviderRequest,
    ) {
        let provider = self.supervisor.thread(id).map(|thread| thread.provider.clone());
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
        match action {
            Action::Input | Action::Steer | Action::Resolve => {}
            Action::Interrupt => {
                // The backstop: a bridge that settled its own turn makes this a no-op.
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.force_settle(now);
                    self.commit(events);
                }
            }
            Action::Release => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let mut events = engine.force_settle(now);
                    events.extend(engine.session_released(Some("released".into()), now));
                    self.commit(events);
                }
            }
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
        if events.is_empty() {
            return;
        }
        if let Err(error) = self.store.append(&events) {
            self.fatal = Some(anyhow::Error::new(error).context("cannot write the journal"));
            return;
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
