use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use qcc_domain::ThreadId;
use tokio::sync::oneshot;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::api::{Contribution, Outcome, PluginRequest, ThreadsAnswer, ThreadsCall};
use crate::bindings::qcc::plugin as imports;
use crate::bindings::{Plugin, wit};
use crate::convert;
use crate::host::EPOCH_TICK;
use crate::manifest::Manifest;
use crate::permissions::Permissions;
use crate::services::{ProcessHandle, ProcessOutput, ProcessSink, Services, SpawnOptions};

/// How long one `handle` call may run before the plugin is considered hung.
const HANDLE_BUDGET: Duration = Duration::from_secs(10);
const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
const MAX_TIMERS: usize = 1024;
const MAX_PROCESSES: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PluginError {
    /// The plugin answered with an error. It keeps running.
    #[error("{0}")]
    Rejected(String),
    /// The plugin trapped or overran its budget. It handles nothing any more.
    #[error("plugin crashed: {0}")]
    Crashed(String),
    #[error("plugin is shut down")]
    Gone,
}

pub(crate) enum Message {
    Call {
        event: wit::Event,
        reply: oneshot::Sender<Result<wit::Outcome, PluginError>>,
    },
    /// Something the host itself produced: process output or exit.
    Internal(wit::Event),
    Shutdown,
}

/// A loaded plugin. Cheap to clone; the plugin shuts down when the last handle drops.
#[derive(Clone)]
pub struct PluginHandle {
    inner: Arc<Inner>,
}

struct Inner {
    manifest: Manifest,
    permissions: Permissions,
    contributions: Vec<Contribution>,
    tx: Sender<Message>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        let _ = self.tx.send(Message::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl PluginHandle {
    pub fn id(&self) -> &str {
        &self.inner.manifest.id
    }

    pub fn manifest(&self) -> &Manifest {
        &self.inner.manifest
    }

    /// The permissions in effect: requested and granted.
    pub fn permissions(&self) -> &Permissions {
        &self.inner.permissions
    }

    pub fn contributions(&self) -> &[Contribution] {
        &self.inner.contributions
    }

    /// Queue a request and wait for the plugin's answer.
    pub async fn call(&self, request: PluginRequest) -> Result<Outcome, PluginError> {
        self.send(request)?.await.map_err(|_| PluginError::Gone)?.map(convert::outcome)
    }

    /// Deliver an event nobody waits on, like process output: the plugin's answer is
    /// only logged.
    pub fn post(&self, request: PluginRequest) -> Result<(), PluginError> {
        self.inner
            .tx
            .send(Message::Internal(convert::event(request)))
            .map_err(|_| PluginError::Gone)
    }

    /// [`PluginHandle::call`] for callers outside an async runtime.
    pub fn call_blocking(&self, request: PluginRequest) -> Result<Outcome, PluginError> {
        self.send(request)?.blocking_recv().map_err(|_| PluginError::Gone)?.map(convert::outcome)
    }

    fn send(
        &self,
        request: PluginRequest,
    ) -> Result<oneshot::Receiver<Result<wit::Outcome, PluginError>>, PluginError> {
        let (reply, receiver) = oneshot::channel();
        self.inner
            .tx
            .send(Message::Call { event: convert::event(request), reply })
            .map_err(|_| PluginError::Gone)?;
        Ok(receiver)
    }
}

/// Compile-independent part of loading: instantiate on a fresh actor thread, run
/// `init`, and hand back the handle.
pub(crate) fn spawn(
    engine: &Engine,
    component: Component,
    manifest: Manifest,
    permissions: Permissions,
    services: Services,
) -> Result<PluginHandle, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let engine = engine.clone();
    let state = State {
        wasi: WasiCtx::builder().inherit_stderr().build(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new().memory_size(MEMORY_LIMIT_BYTES).build(),
        plugin_id: manifest.id.clone(),
        permissions: permissions.clone(),
        services,
        tx: tx.clone(),
        timers: BTreeSet::new(),
        next_timer: 0,
        processes: HashMap::new(),
        next_process: 0,
        deferred: HashMap::new(),
        early: HashMap::new(),
    };
    let thread = std::thread::Builder::new()
        .name(format!("plugin-{}", manifest.id))
        .spawn(move || {
            let mut actor = match Actor::start(&engine, &component, state) {
                Ok((actor, contributions)) => {
                    let _ = ready_tx.send(Ok(contributions));
                    actor
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                    return;
                }
            };
            actor.run(rx);
        })
        .map_err(|error| format!("cannot start plugin thread: {error}"))?;
    let contributions = ready_rx
        .recv()
        .map_err(|_| "plugin thread died during startup".to_owned())??
        .into_iter()
        .map(convert::contribution)
        .collect();
    Ok(PluginHandle {
        inner: Arc::new(Inner { manifest, permissions, contributions, tx, thread: Some(thread) }),
    })
}

/// Store data: everything host imports can reach.
struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    plugin_id: String,
    permissions: Permissions,
    services: Services,
    /// The actor's own queue, for events the host produces.
    tx: Sender<Message>,
    /// Pending timers ordered by deadline.
    timers: BTreeSet<(Instant, u64)>,
    next_timer: u64,
    processes: HashMap<u64, Box<dyn ProcessHandle>>,
    next_process: u64,
    /// Callers waiting for an answer the plugin deferred, by the plugin's token.
    deferred: HashMap<u64, Reply>,
    /// Answers given during the very `handle` call that defers them: the token is
    /// not registered until that call returns. Emptied after every call.
    early: HashMap<u64, Result<wit::Outcome, PluginError>>,
}

type Reply = oneshot::Sender<Result<wit::Outcome, PluginError>>;

impl State {
    fn denied(&self, permission: &str) -> String {
        tracing::warn!(plugin = %self.plugin_id, permission, "permission denied");
        format!("permission denied: {permission}")
    }
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

struct Actor {
    store: Store<State>,
    plugin: Plugin,
    crashed: Option<String>,
}

impl Actor {
    fn start(
        engine: &Engine,
        component: &Component,
        state: State,
    ) -> Result<(Self, Vec<wit::Contribution>), String> {
        let mut linker = Linker::<State>::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|e| e.to_string())?;
        Plugin::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|e| e.to_string())?;

        let mut store = Store::new(engine, state);
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(budget_ticks());
        let plugin = Plugin::instantiate(&mut store, component, &linker)
            .map_err(|error| format!("cannot instantiate: {error:#}"))?;
        let contributions = plugin
            .call_init(&mut store)
            .map_err(|error| format!("init trapped: {error:#}"))?
            .map_err(|error| format!("init failed: {error}"))?;
        Ok((Self { store, plugin, crashed: None }, contributions))
    }

    fn run(&mut self, rx: Receiver<Message>) {
        loop {
            self.fire_due_timers();
            let message = match self.store.data().timers.first() {
                Some((deadline, _)) => {
                    match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                        Ok(message) => message,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match rx.recv() {
                    Ok(message) => message,
                    Err(_) => break,
                },
            };
            match message {
                Message::Call { event, reply } => match self.handle(&event) {
                    Ok(wit::Outcome::Deferred(token)) => self.defer(token, reply),
                    answer => {
                        let _ = reply.send(answer);
                    }
                },
                Message::Internal(event) => self.handle_internal(&event),
                Message::Shutdown => break,
            }
        }
    }

    /// The plugin will answer later. Park the caller until it does.
    fn defer(&mut self, token: u64, reply: Reply) {
        let state = self.store.data_mut();
        if let Some(answer) = state.early.remove(&token) {
            let _ = reply.send(answer);
        } else if let Entry::Vacant(slot) = state.deferred.entry(token) {
            slot.insert(reply);
        } else {
            let _ = reply.send(Err(PluginError::Rejected(format!(
                "plugin deferred two requests with the same token {token}"
            ))));
        }
    }

    fn fire_due_timers(&mut self) {
        loop {
            let now = Instant::now();
            let Some(&(deadline, timer)) = self.store.data().timers.first() else {
                return;
            };
            if deadline > now {
                return;
            }
            self.store.data_mut().timers.remove(&(deadline, timer));
            self.handle_internal(&wit::Event::Timer(timer));
        }
    }

    fn handle_internal(&mut self, event: &wit::Event) {
        if let wit::Event::ProcessExit(exit) = event {
            self.store.data_mut().processes.remove(&exit.process);
        }
        match self.handle(event) {
            Err(PluginError::Rejected(error)) => {
                tracing::warn!(plugin = %self.store.data().plugin_id, %error, "plugin rejected an event");
            }
            Ok(wit::Outcome::Deferred(_)) => {
                tracing::warn!(plugin = %self.store.data().plugin_id, "plugin deferred an event nobody waits on");
            }
            _ => {}
        }
    }

    fn handle(&mut self, event: &wit::Event) -> Result<wit::Outcome, PluginError> {
        if let Some(reason) = &self.crashed {
            return Err(PluginError::Crashed(reason.clone()));
        }
        self.store.set_epoch_deadline(budget_ticks());
        let result = self.plugin.call_handle(&mut self.store, event);
        if !matches!(result, Ok(Ok(wit::Outcome::Deferred(_)))) {
            self.store.data_mut().early.clear();
        }
        match result {
            Ok(Ok(outcome)) => Ok(outcome),
            Ok(Err(message)) => Err(PluginError::Rejected(message)),
            Err(trap) => {
                // A trapped instance may hold broken invariants; it is never entered again.
                // The backtrace goes to the log; the timeline gets the cause.
                let reason = trap.root_cause().to_string();
                let state = self.store.data_mut();
                state.processes.clear();
                state.timers.clear();
                state.early.clear();
                for (_, reply) in state.deferred.drain() {
                    let _ = reply.send(Err(PluginError::Crashed(reason.clone())));
                }
                tracing::error!(plugin = %state.plugin_id, error = %format!("{trap:#}"), "plugin crashed");
                (state.services.on_crash)(&state.plugin_id, &reason);
                self.crashed = Some(reason.clone());
                Err(PluginError::Crashed(reason))
            }
        }
    }
}

fn budget_ticks() -> u64 {
    (HANDLE_BUDGET.as_millis() / EPOCH_TICK.as_millis()) as u64
}

// ----- host imports -----------------------------------------------------------------

impl wit::Host for State {}

impl imports::log::Host for State {
    fn log(&mut self, level: imports::log::Level, message: String) {
        use imports::log::Level;
        let plugin = self.plugin_id.as_str();
        match level {
            Level::Debug => tracing::debug!(plugin, "{message}"),
            Level::Info => tracing::info!(plugin, "{message}"),
            Level::Warn => tracing::warn!(plugin, "{message}"),
            Level::Error => tracing::error!(plugin, "{message}"),
        }
    }
}

impl imports::completions::Host for State {
    fn complete(&mut self, token: u64, answer: Result<wit::Outcome, String>) -> Result<(), String> {
        if matches!(answer, Ok(wit::Outcome::Deferred(_))) {
            return Err("a deferred request cannot be answered with `deferred`".into());
        }
        let answer = answer.map_err(PluginError::Rejected);
        match self.deferred.remove(&token) {
            Some(reply) => {
                // The caller may have stopped waiting; that is its business.
                let _ = reply.send(answer);
            }
            None => {
                self.early.insert(token, answer);
            }
        }
        Ok(())
    }
}

impl imports::deltas::Host for State {
    fn emit(&mut self, thread: String, deltas: Vec<wit::Delta>) -> Result<(), String> {
        if !self.permissions.deltas {
            return Err(self.denied("deltas"));
        }
        // All or nothing: a batch with one malformed delta is refused whole.
        let deltas = deltas.into_iter().map(convert::delta).collect::<Result<Vec<_>, _>>()?;
        (self.services.deltas)(&self.plugin_id, ThreadId(thread), deltas);
        Ok(())
    }
}

impl imports::kv::Host for State {
    fn get(&mut self, key: String) -> Result<Option<Vec<u8>>, String> {
        if !self.permissions.kv {
            return Err(self.denied("kv"));
        }
        self.services.kv.get(&key)
    }

    fn put(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if !self.permissions.kv {
            return Err(self.denied("kv"));
        }
        self.services.kv.put(&key, &value)
    }

    fn remove(&mut self, key: String) -> Result<(), String> {
        if !self.permissions.kv {
            return Err(self.denied("kv"));
        }
        self.services.kv.remove(&key)
    }

    fn keys(&mut self, prefix: String) -> Result<Vec<String>, String> {
        if !self.permissions.kv {
            return Err(self.denied("kv"));
        }
        self.services.kv.keys(&prefix)
    }
}

impl imports::timers::Host for State {
    fn start(&mut self, delay_ms: u64) -> Result<u64, String> {
        if !self.permissions.timers {
            return Err(self.denied("timers"));
        }
        if self.timers.len() >= MAX_TIMERS {
            return Err(format!("more than {MAX_TIMERS} pending timers"));
        }
        self.next_timer += 1;
        let deadline = Instant::now() + Duration::from_millis(delay_ms);
        self.timers.insert((deadline, self.next_timer));
        Ok(self.next_timer)
    }

    fn cancel(&mut self, timer: u64) {
        self.timers.retain(|(_, id)| *id != timer);
    }
}

impl imports::process::Host for State {
    fn spawn(&mut self, options: imports::process::SpawnOptions) -> Result<u64, String> {
        if !self.permissions.allows_process(&options.program) {
            return Err(self.denied(&format!("process {:?}", options.program)));
        }
        if self.processes.len() >= MAX_PROCESSES {
            return Err(format!("more than {MAX_PROCESSES} running processes"));
        }
        self.next_process += 1;
        let process = self.next_process;
        let tx = self.tx.clone();
        let sink: ProcessSink = Arc::new(move |output| {
            let event = match output {
                ProcessOutput::Stdout(data) => {
                    wit::Event::ProcessStdout(wit::ProcessOutput { process, data })
                }
                ProcessOutput::Stderr(data) => {
                    wit::Event::ProcessStderr(wit::ProcessOutput { process, data })
                }
                ProcessOutput::Exit(code) => {
                    wit::Event::ProcessExit(wit::ProcessExit { process, code })
                }
            };
            let _ = tx.send(Message::Internal(event));
        });
        let handle = self.services.processes.spawn(
            SpawnOptions {
                program: options.program,
                args: options.args,
                env: options.env,
                cwd: options.cwd,
            },
            sink,
        )?;
        self.processes.insert(process, handle);
        Ok(process)
    }

    fn write_stdin(&mut self, process: u64, data: Vec<u8>) -> Result<(), String> {
        self.process(process)?.write_stdin(&data)
    }

    fn close_stdin(&mut self, process: u64) -> Result<(), String> {
        self.process(process)?.close_stdin()
    }

    fn kill(&mut self, process: u64) -> Result<(), String> {
        self.process(process)?.kill()
    }
}

impl State {
    fn threads(&self, call: ThreadsCall) -> Result<ThreadsAnswer, String> {
        if !self.permissions.threads {
            return Err(self.denied("threads"));
        }
        (self.services.threads)(&self.plugin_id, call)
    }
}

impl imports::threads::Host for State {
    fn spawn(
        &mut self,
        options: imports::threads::SpawnOptions,
    ) -> Result<wit::ThreadInfo, String> {
        match self.threads(ThreadsCall::Spawn(convert::spawn_options(options)))? {
            ThreadsAnswer::Thread(summary) => Ok(convert::thread_info(*summary)),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn send(&mut self, thread: String, text: String) -> Result<(), String> {
        self.threads(ThreadsCall::Send { thread: ThreadId(thread), text }).map(|_| ())
    }

    fn get(&mut self, thread: String) -> Result<wit::ThreadInfo, String> {
        match self.threads(ThreadsCall::Get(ThreadId(thread)))? {
            ThreadsAnswer::Thread(summary) => Ok(convert::thread_info(*summary)),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn transcript(&mut self, thread: String) -> Result<Vec<imports::threads::Message>, String> {
        match self.threads(ThreadsCall::Transcript(ThreadId(thread)))? {
            ThreadsAnswer::Transcript(entries) => Ok(convert::transcript(entries)),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn watch(&mut self, thread: String) -> Result<(), String> {
        self.threads(ThreadsCall::Watch(ThreadId(thread))).map(|_| ())
    }
}

impl State {
    fn process(&mut self, process: u64) -> Result<&mut Box<dyn ProcessHandle>, String> {
        self.processes.get_mut(&process).ok_or_else(|| format!("no process {process}"))
    }
}
