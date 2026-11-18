//! Conformance kit for provider bridges.
//!
//! [`Harness`] loads a plugin into the real wasm host, drives one thread through it
//! and assembles the deltas with the real assembler — the same path the daemon takes,
//! minus the daemon. On top of that:
//!
//! * [`Harness::conformance`] checks the rules every bridge owes the core;
//! * [`Harness::assert_golden`] compares the assembled timeline with a recorded one;
//! * [`ScriptedProcesses`] replays recorded agent traffic instead of running the agent.

mod scripted;

use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use qcc_domain::{
    Delta, EventPayload, InteractionId, InteractionResolution, Item, ItemShape, PermissionMode,
    ProviderId, RateLimits, Thread, ThreadEvent, ThreadId, TurnStatus, UserInput,
};
use qcc_plugin_host::services::{KvStore, ProcessBackend};
use qcc_plugin_host::{
    Grant, Manifest, PluginError, PluginHandle, PluginHost, PluginRequest, Services, SessionStart,
};
/// What a test asks of a provider beyond a session's own requests, and its answers.
pub use qcc_plugin_host::{Outcome, ProviderRequest};
use qcc_runtime::{GrammarViolation, NewThread, SequentialMint, ThreadEngine, Timeline};

pub use scripted::{ScriptStep, ScriptedProcesses, message_key, script_from_recording};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Configures a [`Harness`].
pub struct HarnessBuilder {
    manifest: Manifest,
    wasm: Vec<u8>,
    grant: Grant,
    provider: Option<ProviderId>,
    kv: Option<Box<dyn KvStore>>,
    processes: Option<Arc<dyn ProcessBackend>>,
}

impl HarnessBuilder {
    /// What the user granted. Defaults to everything the manifest requests.
    pub fn grant(mut self, grant: Grant) -> Self {
        self.grant = grant;
        self
    }

    /// The provider to drive. Defaults to the first one the plugin contributes.
    pub fn provider(mut self, provider: impl Into<ProviderId>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn kv(mut self, kv: impl KvStore + 'static) -> Self {
        self.kv = Some(Box::new(kv));
        self
    }

    /// Replace real processes, usually with [`ScriptedProcesses`].
    pub fn processes(mut self, processes: Arc<dyn ProcessBackend>) -> Self {
        self.processes = Some(processes);
        self
    }

    pub fn build(self) -> Harness {
        let (tx, rx) = std::sync::mpsc::channel();
        let crash = Arc::new(Mutex::new(None));
        let mut services = Services::new(Arc::new(move |_plugin, thread, deltas| {
            let _ = tx.send((thread, deltas));
        }));
        let crash_slot = crash.clone();
        services.on_crash = Arc::new(move |_plugin, reason| {
            *crash_slot.lock().unwrap() = Some(reason.to_owned());
        });
        if let Some(kv) = self.kv {
            services.kv = kv;
        }
        if let Some(processes) = self.processes {
            services.processes = processes;
        }

        let mut host = PluginHost::new().expect("wasm engine");
        let plugin =
            host.load(self.manifest, &self.wasm, &self.grant, services).expect("plugin loads");
        let provider = self.provider.unwrap_or_else(|| {
            host.providers().next().expect("the plugin contributes a provider").1.id.clone()
        });

        let thread_id = ThreadId::new("thread-1");
        let thread = NewThread {
            id: thread_id.clone(),
            provider: provider.clone(),
            cwd: "/work".into(),
            environment: None,
            model: None,
            effort: None,
            parent: None,
            project: None,
            permission_mode: None,
            skills_enabled: Vec::new(),
            skills_disabled: Vec::new(),
            started_by: None,
            hidden: false,
            automation: None,
        }
        .into_thread(0);
        let (engine, events) = ThreadEngine::create(thread, Box::new(SequentialMint::default()), 0);
        Harness {
            _host: host,
            plugin,
            provider,
            thread_id,
            engine,
            rx,
            events,
            violations: Vec::new(),
            crash,
            foreign_deltas: 0,
            rate_limits: Vec::new(),
            model: None,
            effort: None,
            permission_mode: None,
        }
    }
}

/// One thread driven through one provider of one plugin.
pub struct Harness {
    // Keeps the engine's watchdog alive for as long as the plugin runs.
    _host: PluginHost,
    plugin: PluginHandle,
    provider: ProviderId,
    thread_id: ThreadId,
    engine: ThreadEngine,
    rx: Receiver<(ThreadId, Vec<Delta>)>,
    events: Vec<ThreadEvent>,
    violations: Vec<GrammarViolation>,
    crash: Arc<Mutex<Option<String>>>,
    foreign_deltas: usize,
    /// Reports of the account's usage limits, which the daemon keeps per provider
    /// and never puts in the timeline.
    rate_limits: Vec<RateLimits>,
    model: Option<String>,
    effort: Option<String>,
    permission_mode: Option<PermissionMode>,
}

impl Harness {
    /// A plugin shipped in this repository, by its directory name under `plugins/`.
    pub fn bundled(plugin_id: &str) -> HarnessBuilder {
        let plugin = qcc_plugins_bundle::plugin(plugin_id)
            .unwrap_or_else(|| panic!("no bundled plugin {plugin_id:?}"));
        Self::plugin(plugin.manifest, plugin.wasm)
    }

    /// Any plugin, from its manifest text and component bytes.
    pub fn plugin(manifest: &str, wasm: &[u8]) -> HarnessBuilder {
        HarnessBuilder {
            manifest: Manifest::parse(manifest).expect("valid manifest"),
            wasm: wasm.to_vec(),
            grant: Grant::AllRequested,
            provider: None,
            kv: None,
            processes: None,
        }
    }

    pub fn handle(&self) -> &PluginHandle {
        &self.plugin
    }

    pub fn thread(&self) -> &Thread {
        self.engine.thread()
    }

    pub fn thread_id(&self) -> &ThreadId {
        &self.thread_id
    }

    /// The assembled timeline so far.
    pub fn events(&self) -> &[ThreadEvent] {
        &self.events
    }

    /// Why the plugin crashed, if it did.
    pub fn crash(&self) -> Option<String> {
        self.crash.lock().unwrap().clone()
    }

    /// The model the next session start asks for.
    pub fn set_model(&mut self, model: Option<&str>) {
        self.model = model.map(str::to_owned);
    }

    /// The effort the next session start asks for.
    pub fn set_effort(&mut self, effort: Option<&str>) {
        self.effort = effort.map(str::to_owned);
    }

    /// The permission mode the next session start asks for.
    pub fn set_permission_mode(&mut self, mode: Option<PermissionMode>) {
        self.permission_mode = mode;
    }

    /// Start a session, optionally with a first prompt, the way the daemon does.
    pub fn start(&mut self, prompt: Option<&str>) -> Result<(), PluginError> {
        self.start_with(prompt, None)
    }

    /// Start a session resuming `provider_session`.
    pub fn resume(&mut self, prompt: Option<&str>) -> Result<(), PluginError> {
        let handle = self.engine.thread().provider_session.clone();
        self.start_with(prompt, handle)
    }

    fn start_with(
        &mut self,
        prompt: Option<&str>,
        resume: Option<String>,
    ) -> Result<(), PluginError> {
        if let Some(prompt) = prompt {
            let events = self.engine.submit_input(prompt.into(), 0);
            self.events.extend(events);
        }
        let resumed = resume.is_some();
        let outcome = self.call(ProviderRequest::Start(Box::new(SessionStart {
            thread: self.thread_id.clone(),
            cwd: "/work".into(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            resume,
            prompt: prompt.map(UserInput::from),
            permission_mode: self.permission_mode,
            env: vec![("QCC_THREAD_ID".into(), self.thread_id.to_string())],
            skills: Vec::new(),
            extra_dirs: Vec::new(),
            mcp_servers: Vec::new(),
        })));
        match outcome {
            Ok(Outcome::SessionStarted { provider_session }) => {
                let events = self.engine.session_started(provider_session, resumed, 0);
                self.events.extend(events);
                self.pump();
                Ok(())
            }
            Ok(other) => panic!("start answered {other:?}"),
            Err(error) => {
                let events = self.engine.session_failed(error.to_string(), 0);
                self.events.extend(events);
                Err(error)
            }
        }
    }

    pub fn input(&mut self, text: &str) -> Result<(), PluginError> {
        self.input_with(text.into())
    }

    /// Send a message with everything a message can carry: notes, attachments.
    pub fn input_with(&mut self, input: UserInput) -> Result<(), PluginError> {
        let events = self.engine.submit_input(input.clone(), 0);
        self.events.extend(events);
        let thread = self.thread_id.clone();
        self.call(ProviderRequest::Input { thread, input })?;
        self.pump();
        Ok(())
    }

    /// Feed a message into the running turn.
    pub fn steer(&mut self, text: &str) -> Result<(), PluginError> {
        let events = self.engine.steer_input(text.into(), 0);
        self.events.extend(events);
        let thread = self.thread_id.clone();
        self.call(ProviderRequest::Steer { thread, input: text.into() })?;
        self.pump();
        Ok(())
    }

    pub fn interrupt(&mut self) -> Result<(), PluginError> {
        let thread = self.thread_id.clone();
        self.call(ProviderRequest::Interrupt { thread })?;
        self.pump();
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), PluginError> {
        let thread = self.thread_id.clone();
        self.call(ProviderRequest::Stop { thread })?;
        self.pump();
        Ok(())
    }

    /// Answer the oldest pending interaction.
    pub fn resolve(&mut self, resolution: InteractionResolution) -> Result<(), PluginError> {
        let interaction_id = self.pending_interaction().expect("an interaction is pending");
        let (request_id, events) = self
            .engine
            .resolve_interaction(interaction_id, resolution.clone(), 0)
            .expect("pending interaction");
        self.events.extend(events);
        let thread = self.thread_id.clone();
        self.call(ProviderRequest::ResolveInteraction { thread, request_id, resolution })?;
        self.pump();
        Ok(())
    }

    pub fn pending_interaction(&self) -> Option<InteractionId> {
        let payloads = self.events.iter().map(|event| &event.payload);
        let mut timeline = Timeline::default();
        for payload in payloads {
            let _ = timeline.apply(payload);
        }
        timeline.pending_interactions().first().cloned()
    }

    /// Send any provider request to the plugin.
    pub fn call(&mut self, request: ProviderRequest) -> Result<Outcome, PluginError> {
        self.plugin
            .call_blocking(PluginRequest::Provider { provider: self.provider.clone(), request })
    }

    /// Assemble every delta the plugin has emitted so far.
    pub fn pump(&mut self) {
        // What the plugin's processes said may still be in its queue, and handling it
        // can make a scripted process say more. Wait until a round brings nothing
        // new, or what is assembled depends on how fast the plugin thread runs.
        loop {
            self.plugin.flush_blocking();
            let mut quiet = true;
            while let Ok((thread, deltas)) = self.rx.try_recv() {
                self.apply(thread, deltas);
                quiet = false;
            }
            if quiet {
                return;
            }
        }
    }

    /// Keep assembling until `done` holds. Panics after ten seconds.
    pub fn wait_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + DEFAULT_TIMEOUT;
        loop {
            self.pump();
            if done(self) {
                return;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            match self.rx.recv_timeout(left.min(Duration::from_millis(20))) {
                Ok((thread, deltas)) => self.apply(thread, deltas),
                Err(RecvTimeoutError::Timeout) if !left.is_zero() => {}
                Err(_) => panic!(
                    "timed out waiting for {what}; timeline so far:\n{}",
                    self.rendered().join("\n")
                ),
            }
        }
    }

    /// Wait until `count` turns have settled.
    pub fn wait_turns(&mut self, count: usize) {
        self.wait_until(&format!("{count} settled turn(s)"), |harness| {
            harness.count(|payload| matches!(payload, EventPayload::TurnCompleted { .. })) >= count
        });
    }

    /// Text of every settled agent message, in order.
    pub fn agent_messages(&self) -> Vec<String> {
        self.completed_items()
            .into_iter()
            .filter_map(|item| match item.shape {
                ItemShape::AgentMessage { text } => Some(text),
                _ => None,
            })
            .collect()
    }

    /// Every settled item, in order.
    pub fn completed_items(&self) -> Vec<Item> {
        self.events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::ItemCompleted { item, .. } => Some(item.clone()),
                _ => None,
            })
            .collect()
    }

    /// How each settled turn ended, in order.
    pub fn turn_statuses(&self) -> Vec<(TurnStatus, Option<String>)> {
        self.events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::TurnCompleted { status, error, .. } => Some((*status, error.clone())),
                _ => None,
            })
            .collect()
    }

    /// The usage-limit reports the bridge made, in order.
    pub fn rate_limits(&self) -> &[RateLimits] {
        &self.rate_limits
    }

    pub fn count(&self, matches: impl Fn(&EventPayload) -> bool) -> usize {
        self.events.iter().filter(|event| matches(&event.payload)).count()
    }

    fn apply(&mut self, thread: ThreadId, deltas: Vec<Delta>) {
        if thread != self.thread_id {
            self.foreign_deltas += deltas.len();
            return;
        }
        let (limits, deltas): (Vec<Delta>, Vec<Delta>) =
            deltas.into_iter().partition(|delta| matches!(delta, Delta::RateLimits { .. }));
        self.rate_limits.extend(limits.into_iter().filter_map(|delta| match delta {
            Delta::RateLimits { limits } => Some(limits),
            _ => None,
        }));
        let events = self.engine.apply_deltas(deltas, 0);
        self.events.extend(events);
        self.violations.extend(self.engine.take_violations());
    }

    /// The rules every bridge owes the core. Returns one line per broken rule.
    ///
    /// * `grammar` — no delta was refused by the assembler;
    /// * `timeline` — the assembled events satisfy the timeline grammar;
    /// * `settled` — nothing is left open: every turn the bridge opened reached a
    ///   terminal state and no interaction is pending;
    /// * `scoped` — no delta named a thread the plugin was not given.
    pub fn conformance(&self) -> Vec<String> {
        let mut broken = Vec::new();
        for violation in &self.violations {
            broken.push(format!("grammar: {violation}"));
        }
        let mut timeline = Timeline::default();
        for (index, event) in self.events.iter().enumerate() {
            if let Err(violation) = timeline.apply(&event.payload) {
                broken.push(format!("timeline: event {}: {violation}", index + 1));
            }
            if event.seq != index as u64 + 1 {
                broken.push(format!("timeline: event {} has seq {}", index + 1, event.seq));
            }
        }
        if let Some(turn) = timeline.open_turn() {
            broken.push(format!("settled: turn {turn} never reached a terminal state"));
        }
        for interaction in timeline.pending_interactions() {
            broken.push(format!("settled: interaction {interaction} is still pending"));
        }
        if self.foreign_deltas > 0 {
            broken.push(format!("scoped: {} delta(s) for an unknown thread", self.foreign_deltas));
        }
        broken
    }

    pub fn assert_conformant(&self) {
        let broken = self.conformance();
        assert!(
            broken.is_empty(),
            "bridge breaks the contract:\n  {}\ntimeline:\n{}",
            broken.join("\n  "),
            self.rendered().join("\n")
        );
    }

    /// The timeline as one JSON object per event, without sequence numbers and
    /// timestamps. Ids are deterministic (`turn_1`, `item_1`, …).
    pub fn rendered(&self) -> Vec<String> {
        self.events
            .iter()
            .map(|event| serde_json::to_string(&event.payload).expect("events serialize"))
            .collect()
    }

    /// Compare the timeline with a recorded one. Run with `QCC_UPDATE_GOLDEN=1` to
    /// record.
    pub fn assert_golden(&self, path: impl AsRef<Path>) {
        let path = path.as_ref();
        let actual = self.rendered().join("\n") + "\n";
        if std::env::var_os("QCC_UPDATE_GOLDEN").is_some() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("golden directory");
            }
            std::fs::write(path, &actual).expect("write golden file");
            return;
        }
        let expected = std::fs::read_to_string(path).unwrap_or_else(|error| {
            panic!("{}: {error}; record it with QCC_UPDATE_GOLDEN=1", path.display())
        });
        if expected != actual {
            let line = expected.lines().zip(actual.lines()).position(|(e, a)| e != a);
            let line = line.unwrap_or(expected.lines().count().min(actual.lines().count()));
            panic!(
                "timeline differs from {} at line {}:\n  expected: {}\n  actual:   {}\n\
                 re-record with QCC_UPDATE_GOLDEN=1 if the change is intended",
                path.display(),
                line + 1,
                expected.lines().nth(line).unwrap_or("<end>"),
                actual.lines().nth(line).unwrap_or("<end>"),
            );
        }
    }
}
