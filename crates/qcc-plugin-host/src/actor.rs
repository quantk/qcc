use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use qcc_domain::{CommentId, ThreadId};
use tokio::sync::oneshot;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::api::{Contribution, FileRoot, Outcome, PluginRequest, ThreadsAnswer, ThreadsCall};
use crate::bindings::qcc::plugin as imports;
use crate::bindings::{Plugin, wit};
use crate::convert;
use crate::host::EPOCH_TICK;
use crate::manifest::Manifest;
use crate::permissions::Permissions;
use crate::services::{ProcessHandle, ProcessOutput, ProcessSink, Services, SpawnOptions};

/// How long one `handle` call may run before the plugin is considered hung.
const HANDLE_BUDGET: Duration = Duration::from_secs(10);
/// Linear memory of all the plugin's instances together.
const MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
/// Elements of all the plugin's tables together.
const TABLE_ELEMENTS_LIMIT: usize = 1_000_000;
/// A component is a handful of core instances, memories and tables (its own code, the
/// WASI adapter, shims); these leave room for that and nothing like wasmtime's
/// default of ten thousand each.
const MAX_INSTANCES: usize = 64;
const MAX_MEMORIES: usize = 16;
const MAX_TABLES: usize = 64;
const MAX_TIMERS: usize = 1024;
const MAX_PROCESSES: usize = 64;
/// Process output queued for the plugin and not yet handled. Past this a process
/// waits until the plugin catches up; its pipe then fills and the child waits too.
const MAX_OUTPUT_IN_FLIGHT: usize = 8 * 1024 * 1024;
/// Callers a plugin may keep waiting at once.
const MAX_DEFERRED: usize = 1024;
/// Answers a plugin may give inside one call for requests that call defers. More
/// than one is already unusual; the cap keeps a plugin from piling host memory up.
const MAX_EARLY: usize = 64;

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
    /// Answered once everything queued before it is handled.
    Flush(oneshot::Sender<()>),
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

    /// Whether `provider`, one of this plugin's, declared that it looks at images.
    fn takes_images(&self, provider: &str) -> bool {
        self.inner.contributions.iter().any(|contribution| {
            matches!(contribution, Contribution::Provider(info)
                if info.id.as_str() == provider && info.capabilities.images)
        })
    }

    /// Queue a request and wait for the plugin's answer.
    pub async fn call(&self, request: PluginRequest) -> Result<Outcome, PluginError> {
        self.send(request)?.await.map_err(|_| PluginError::Gone)?.map(convert::outcome)
    }

    /// Deliver an event nobody waits on, like process output: the plugin's answer is
    /// only logged.
    pub fn post(&self, request: PluginRequest) -> Result<(), PluginError> {
        // Every turn of every thread is not news for a plugin that did not ask.
        if matches!(request, PluginRequest::TurnEnded { .. }) && !self.inner.permissions.turn_events
        {
            return Ok(());
        }
        if matches!(request, PluginRequest::CommentsChanged(_)) && !self.inner.permissions.comments
        {
            return Ok(());
        }
        self.inner
            .tx
            .send(Message::Internal(convert::event(request, |id| self.takes_images(id))))
            .map_err(|_| PluginError::Gone)
    }

    /// [`PluginHandle::call`] for callers outside an async runtime.
    pub fn call_blocking(&self, request: PluginRequest) -> Result<Outcome, PluginError> {
        self.send(request)?.blocking_recv().map_err(|_| PluginError::Gone)?.map(convert::outcome)
    }

    /// Stop the plugin once it has handled what is already queued, without waiting
    /// for the last handle to drop. Its processes are killed and callers still
    /// waiting on a deferred answer hear [`PluginError::Gone`]; a plugin turned off
    /// must not keep an agent running because a call holds a handle.
    pub(crate) fn stop(&self) {
        let _ = self.inner.tx.send(Message::Shutdown);
    }

    /// Wait until the plugin has handled everything queued so far, its own process
    /// output and timers included. For callers that must see a quiet plugin, like
    /// a deterministic test.
    pub fn flush_blocking(&self) {
        let (done, wait) = oneshot::channel();
        if self.inner.tx.send(Message::Flush(done)).is_ok() {
            let _ = wait.blocking_recv();
        }
    }

    fn send(
        &self,
        request: PluginRequest,
    ) -> Result<oneshot::Receiver<Result<wit::Outcome, PluginError>>, PluginError> {
        let (reply, receiver) = oneshot::channel();
        self.inner
            .tx
            .send(Message::Call {
                event: convert::event(request, |id| self.takes_images(id)),
                reply,
            })
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
        limits: Limits::default(),
        plugin_id: manifest.id.clone(),
        permissions: permissions.clone(),
        services,
        tx: tx.clone(),
        timers: BTreeSet::new(),
        next_timer: 0,
        processes: HashMap::new(),
        next_process: 0,
        next_request: 0,
        deferred: HashMap::new(),
        early: HashMap::new(),
        output: Arc::default(),
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
    limits: Limits,
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
    /// The last id given out by `threads.diff` or `threads.models`.
    next_request: u64,
    /// Callers waiting for an answer the plugin deferred, by the plugin's token.
    deferred: HashMap<u64, Reply>,
    /// Answers given during the very `handle` call that defers them: the token is
    /// not registered until that call returns. Emptied after every call.
    early: HashMap<u64, Result<wit::Outcome, PluginError>>,
    /// Process output sent to the actor's queue and not handled yet.
    output: Arc<OutputBudget>,
}

/// Bytes of process output on their way to the plugin, shared by the threads that
/// read the processes and the actor that hands the output on.
#[derive(Debug, Default)]
struct OutputBudget {
    state: Mutex<(usize, bool)>,
    drained: Condvar,
}

impl OutputBudget {
    /// Wait for room for `bytes` more, then take it. Returns at once when the plugin
    /// is gone. A chunk larger than the whole budget still goes alone.
    fn take(&self, bytes: usize) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        while !state.1 && state.0 > 0 && state.0 + bytes > MAX_OUTPUT_IN_FLIGHT {
            state = self.drained.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        state.0 += bytes;
    }

    fn give_back(&self, bytes: usize) {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.0 = state.0.saturating_sub(bytes);
        self.drained.notify_all();
    }

    /// Nobody will hand output on any more: let every waiting reader go.
    fn close(&self) {
        self.state.lock().unwrap_or_else(|p| p.into_inner()).1 = true;
        self.drained.notify_all();
    }
}

type Reply = oneshot::Sender<Result<wit::Outcome, PluginError>>;

impl State {
    fn denied(&self, permission: &str) -> String {
        tracing::warn!(plugin = %self.plugin_id, permission, "permission denied");
        format!("permission denied: {permission}")
    }
}

/// Resource limits on everything a plugin can grow inside its store.
///
/// Memories and tables are counted together, not each on its own: a component may
/// hold several, and a cap per memory would multiply by their number. Nothing is
/// ever freed before the store goes, so the totals only grow.
#[derive(Debug, Default)]
struct Limits {
    memory: usize,
    table_elements: usize,
}

impl Limits {
    fn grow(used: &mut usize, current: usize, desired: usize, limit: usize) -> bool {
        let total = (*used - current.min(*used)).saturating_add(desired);
        if total > limit {
            return false;
        }
        *used = total;
        true
    }
}

impl ResourceLimiter for Limits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(Self::grow(&mut self.memory, current, desired, MEMORY_LIMIT_BYTES))
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(Self::grow(&mut self.table_elements, current, desired, TABLE_ELEMENTS_LIMIT))
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
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
                Message::Internal(event) => {
                    let bytes = match &event {
                        wit::Event::ProcessStdout(output) | wit::Event::ProcessStderr(output) => {
                            output.data.len()
                        }
                        _ => 0,
                    };
                    self.handle_internal(&event);
                    if bytes > 0 {
                        self.store.data().output.give_back(bytes);
                    }
                }
                Message::Flush(done) => {
                    let _ = done.send(());
                }
                Message::Shutdown => break,
            }
        }
        self.store.data().output.close();
    }

    /// The plugin will answer later. Park the caller until it does.
    fn defer(&mut self, token: u64, reply: Reply) {
        let state = self.store.data_mut();
        // Only the answer to this very call can be waiting here; any other one names a
        // token nobody deferred, and kept it could answer a later, unrelated call.
        let early = state.early.remove(&token);
        state.early.clear();
        if let Some(answer) = early {
            let _ = reply.send(answer);
            return;
        }
        // Callers that gave up waiting (a daemon-side timeout) free their place.
        state.deferred.retain(|_, reply| !reply.is_closed());
        if state.deferred.len() >= MAX_DEFERRED {
            let _ = reply.send(Err(PluginError::Rejected(format!(
                "plugin keeps more than {MAX_DEFERRED} requests waiting"
            ))));
        } else if let Entry::Vacant(slot) = state.deferred.entry(token) {
            slot.insert(reply);
        } else {
            let _ = reply.send(Err(PluginError::Rejected(format!(
                "plugin deferred two requests with the same token {token}"
            ))));
        }
    }

    /// Fire the timers that are due now. Ones the plugin starts meanwhile wait for the
    /// next round, even with no delay: a plugin restarting a zero timer from
    /// `on_timer` must not keep the actor from its queue.
    fn fire_due_timers(&mut self) {
        let now = Instant::now();
        // Timer ids only grow, so this also leaves out a new timer due at `now` itself.
        let newest = self.store.data().next_timer;
        loop {
            let Some(&(deadline, timer)) = self.store.data().timers.first() else {
                return;
            };
            if deadline > now || timer > newest {
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
                state.output.close();
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
                if self.early.len() >= MAX_EARLY && !self.early.contains_key(&token) {
                    return Err(format!(
                        "more than {MAX_EARLY} answers to requests not yet deferred"
                    ));
                }
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
        let budget = self.output.clone();
        let sink: ProcessSink = Arc::new(move |output| {
            let event = match output {
                ProcessOutput::Stdout(data) => {
                    budget.take(data.len());
                    wit::Event::ProcessStdout(wit::ProcessOutput { process, data })
                }
                ProcessOutput::Stderr(data) => {
                    budget.take(data.len());
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

    fn diff(&mut self, thread: String) -> Result<u64, String> {
        let request = self.next_request + 1;
        self.threads(ThreadsCall::Diff { thread: ThreadId(thread), request })?;
        self.next_request = request;
        Ok(request)
    }

    fn set_skill(&mut self, thread: String, skill: String, enabled: bool) -> Result<(), String> {
        self.threads(ThreadsCall::SetSkill { thread: ThreadId(thread), skill, enabled }).map(|_| ())
    }

    fn changes(&mut self, thread: String) -> Result<u64, String> {
        let request = self.next_request + 1;
        self.threads(ThreadsCall::Changes { thread: ThreadId(thread), request })?;
        self.next_request = request;
        Ok(request)
    }

    fn models(&mut self, provider: String, refresh: bool) -> Result<u64, String> {
        let request = self.next_request + 1;
        self.threads(ThreadsCall::Models { provider: provider.into(), refresh, request })?;
        self.next_request = request;
        Ok(request)
    }

    fn providers(&mut self) -> Result<Vec<wit::ProviderInfo>, String> {
        match self.threads(ThreadsCall::Providers)? {
            ThreadsAnswer::Providers(providers) => {
                Ok(providers.into_iter().map(convert::provider_info).collect())
            }
            other => Err(format!("unexpected answer {other:?}")),
        }
    }
}

impl State {
    /// A `comments` call, which goes to the core the way `threads` calls do.
    fn comments(&self, call: ThreadsCall) -> Result<ThreadsAnswer, String> {
        if !self.permissions.comments {
            return Err(self.denied("comments"));
        }
        (self.services.threads)(&self.plugin_id, call)
    }
}

impl State {
    /// A `files` call, which the core answers: it knows where threads work.
    fn files(&self, call: ThreadsCall) -> Result<ThreadsAnswer, String> {
        if !self.permissions.files {
            return Err(self.denied("files"));
        }
        (self.services.threads)(&self.plugin_id, call)
    }
}

fn file_root(root: imports::files::Root) -> FileRoot {
    match root {
        imports::files::Root::Thread(thread) => FileRoot::Thread(ThreadId(thread)),
        imports::files::Root::Own => FileRoot::Own,
    }
}

impl imports::files::Host for State {
    fn read(&mut self, root: imports::files::Root, path: String) -> Result<Option<String>, String> {
        match self.files(ThreadsCall::ReadFile { root: file_root(root), path })? {
            ThreadsAnswer::File(text) => Ok(text),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn list(&mut self, root: imports::files::Root, dir: String) -> Result<Vec<String>, String> {
        match self.files(ThreadsCall::ListFiles { root: file_root(root), dir })? {
            ThreadsAnswer::Names(names) => Ok(names),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }
}

impl imports::views::Host for State {
    fn refresh(&mut self, view: String, thread: Option<String>) -> Result<(), String> {
        // No permission: a plugin can only ask the app to redraw its own views.
        let call = ThreadsCall::RefreshView { view, thread: thread.map(ThreadId) };
        (self.services.threads)(&self.plugin_id, call).map(|_| ())
    }
}

impl imports::comments::Host for State {
    fn add(
        &mut self,
        comment: imports::comments::NewComment,
    ) -> Result<imports::comments::LineComment, String> {
        let comment = convert::new_comment(&self.plugin_id, comment);
        match self.comments(ThreadsCall::AddComment(comment))? {
            ThreadsAnswer::Comment(comment) => Ok(convert::line_comment(*comment)),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn resolve(
        &mut self,
        ids: Vec<String>,
        resolution: Option<String>,
        reopen: bool,
    ) -> Result<(), String> {
        let ids = ids.into_iter().map(CommentId::new).collect();
        self.comments(ThreadsCall::ResolveComments { ids, resolution, reopen }).map(|_| ())
    }

    fn send(&mut self, thread: String, ids: Vec<String>, text: String) -> Result<(), String> {
        // A message to an agent is what the `threads` permission is about.
        if !self.permissions.threads {
            return Err(self.denied("threads"));
        }
        let ids = ids.into_iter().map(CommentId::new).collect();
        self.comments(ThreadsCall::SendComments { thread: ThreadId(thread), ids, text }).map(|_| ())
    }

    fn list(
        &mut self,
        thread: String,
        include_resolved: bool,
    ) -> Result<Vec<imports::comments::LineComment>, String> {
        let call = ThreadsCall::ListComments { thread: ThreadId(thread), include_resolved };
        match self.comments(call)? {
            ThreadsAnswer::Comments(comments) => {
                Ok(comments.into_iter().map(convert::line_comment).collect())
            }
            other => Err(format!("unexpected answer {other:?}")),
        }
    }
}

impl imports::notifications::Host for State {
    fn show(&mut self, notification: imports::notifications::Notification) -> Result<(), String> {
        if !self.permissions.notifications {
            return Err(self.denied("notifications"));
        }
        (self.services.notifications)(&self.plugin_id, convert::notification(notification));
        Ok(())
    }
}

impl State {
    fn process(&mut self, process: u64) -> Result<&mut Box<dyn ProcessHandle>, String> {
        self.processes.get_mut(&process).ok_or_else(|| format!("no process {process}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_output_waits_for_the_plugin_to_catch_up() {
        let budget = Arc::new(OutputBudget::default());
        budget.take(MAX_OUTPUT_IN_FLIGHT);
        let reader = {
            let budget = budget.clone();
            std::thread::spawn(move || budget.take(1))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(!reader.is_finished(), "a full budget let more output through");
        budget.give_back(1);
        reader.join().unwrap();

        // A plugin that goes away releases everyone still waiting.
        let blocked = {
            let budget = budget.clone();
            std::thread::spawn(move || budget.take(MAX_OUTPUT_IN_FLIGHT))
        };
        budget.close();
        blocked.join().unwrap();
    }

    #[test]
    fn memory_is_limited_across_all_memories_of_a_plugin() {
        let mut limits = Limits::default();
        let half = MEMORY_LIMIT_BYTES / 2;
        assert!(limits.memory_growing(0, half, None).unwrap());
        assert!(limits.memory_growing(0, half, None).unwrap(), "a second memory fits in the rest");
        assert!(!limits.memory_growing(0, 1, None).unwrap(), "a third one does not");
        // Growing an existing memory counts only what it adds.
        let mut limits = Limits::default();
        assert!(limits.memory_growing(0, half, None).unwrap());
        assert!(limits.memory_growing(half, MEMORY_LIMIT_BYTES, None).unwrap());
        assert!(!limits.memory_growing(MEMORY_LIMIT_BYTES, MEMORY_LIMIT_BYTES + 1, None).unwrap());
    }
}
