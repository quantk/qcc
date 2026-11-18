//! Provider sessions: starting them, calling providers and taking their deltas.

use std::collections::BTreeMap;

use qcc_domain::{
    Delta, EnvironmentState, EventPayload, McpServer, ProviderId, Role, ThreadId, UserInput,
};
use qcc_plugin_host::{Outcome, PluginError, PluginRequest, ProviderRequest, SessionStart};
use qcc_protocol::methods::{Empty, McpServerSettings};
use qcc_protocol::paths;

use super::{Action, Command, Core, Responder, now_ms};
use crate::handover;

impl Core {
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

    /// Ask the thread's provider to start a session, resuming the previous one when
    /// the provider can. The answer comes back as [`Command::SessionStartFinished`].
    pub(super) fn start_session(&mut self, id: &ThreadId, prompt: Option<UserInput>) {
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
        let mcp_servers = mcp_servers_for(&self.settings.mcp_servers, &thread.mcp_disabled);
        let resume = thread.provider_session.filter(|_| info.capabilities.resume);
        let resumed = resume.is_some();
        let prompt = match prompt {
            Some(UserInput { text, notes, attachments }) if thread.handover => {
                Some(UserInput { text: self.handover_prompt(id, text), notes, attachments })
            }
            prompt => prompt,
        };
        let request = PluginRequest::Provider {
            provider: thread.provider,
            request: ProviderRequest::Start(Box::new(SessionStart {
                thread: id.clone(),
                cwd: thread.cwd,
                model: thread.model,
                effort: thread.effort,
                resume,
                prompt,
                permission_mode: thread.permission_mode,
                env: self.session_env(id),
                skills: self.session_skills(&thread.skills_enabled, &thread.skills_disabled),
                extra_dirs,
                mcp_servers,
            })),
        };
        let session = self.sessions.entry(id.clone()).or_default();
        session.starting = true;
        // The new session is told everything from the start.
        session.notes.clear();
        let (plugin, commands, thread) = (plugin.clone(), self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = plugin.call(request).await;
            let _ = commands.send(Command::SessionStartFinished { thread, resumed, result });
        });
    }

    /// The qcc MCP servers the thread's session should have now.
    pub(super) fn thread_mcp(&self, id: &ThreadId) -> Vec<McpServer> {
        self.supervisor
            .thread(id)
            .map(|thread| mcp_servers_for(&self.settings.mcp_servers, &thread.mcp_disabled))
            .unwrap_or_default()
    }

    /// The thread's qcc MCP servers were `before` and may be other now: bring its
    /// session along. A provider that changes them in a running session is told at
    /// once; another one's session is released when no turn runs, and the next
    /// message resumes it with the change. A thread without a session gets the
    /// servers when one starts.
    pub(super) fn mcp_changed(&mut self, id: &ThreadId, before: &[McpServer]) {
        if self.thread_mcp(id) == before {
            return;
        }
        let Some(thread) = self.supervisor.thread(id) else { return };
        let live_update = self
            .host
            .provider(&thread.provider)
            .is_some_and(|(_, info)| info.capabilities.mcp_live);
        if live_update {
            self.push_mcp(id);
        } else if self.supervisor.engine_ref(id).is_some_and(|engine| engine.session_live())
            || self.in_transition(id)
        {
            self.sessions.entry(id.clone()).or_default().release_when_idle = true;
        }
    }

    /// Send the thread's qcc MCP servers to its running session, or remember to once
    /// the session can take them. Each update carries the whole set, so one at a
    /// time is enough for the last to win.
    fn push_mcp(&mut self, id: &ThreadId) {
        let live = self.supervisor.engine_ref(id).is_some_and(|engine| engine.session_live());
        let session = self.sessions.entry(id.clone()).or_default();
        if session.starting || session.stopping || session.mcp_updating {
            session.mcp_stale = true;
            return;
        }
        session.mcp_stale = false;
        // A session that starts later is given the servers at its start.
        if !live {
            return;
        }
        session.mcp_updating = true;
        let servers = self.thread_mcp(id);
        self.call_provider(id, Action::UpdateMcp, None, |thread| ProviderRequest::SetMcpServers {
            thread,
            servers,
        });
    }

    /// Send a change of the MCP servers the session could not take earlier.
    fn push_stale_mcp(&mut self, id: &ThreadId) {
        if self.sessions.get(id).is_some_and(|session| session.mcp_stale) {
            self.push_mcp(id);
        }
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
        // Since the last session the thread either got another agent or moved into
        // a directory of its own; the newer of the two says why it starts afresh.
        let latest_session = events
            .iter()
            .rposition(|event| matches!(event.payload, EventPayload::SessionStarted { .. }))
            .unwrap_or(0);
        let moved = events[latest_session..]
            .iter()
            .rev()
            .find_map(|event| match &event.payload {
                EventPayload::EnvironmentAttached { .. } => Some(true),
                EventPayload::AgentChanged { .. } => Some(false),
                _ => None,
            })
            .unwrap_or(false);
        let reason = if moved {
            let cwd =
                self.supervisor.thread(id).map(|thread| thread.cwd.clone()).unwrap_or_default();
            handover::Reason::Moved { cwd }
        } else {
            handover::Reason::OtherAgent(previous.map_or_else(
                || "another agent".to_owned(),
                |provider| {
                    self.host
                        .provider(&provider)
                        .map_or(provider.to_string(), |(_, info)| info.name.clone())
                },
            ))
        };
        let mut history = self.store.transcript(id, false).unwrap_or_default();
        // The message itself was journaled before the session was asked to start.
        if history.last().is_some_and(|last| last.role == Role::User && last.text == message) {
            history.pop();
        }
        handover::prompt(&reason, &history, &message)
    }

    pub(super) fn session_start_finished(
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
        // A change that came while the session started is not in it. `take` left
        // a default entry in place of the one taken.
        let next = self.sessions.get_mut(&id).expect("the entry stays in the map");
        next.release_when_idle = session.release_when_idle;
        next.notes = session.notes;
        next.mcp_stale = session.mcp_stale;
        match result {
            Ok(Outcome::SessionStarted { provider_session }) => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.session_started(provider_session, resumed, now_ms());
                    self.commit(events);
                }
                self.apply_deltas(&id, session.buffered);
                // A change of the MCP servers that came while it started.
                self.push_stale_mcp(&id);
                // The session was started to be compacted.
                if session.compact_after_start {
                    self.call_provider(&id, Action::Compact, None, |thread| {
                        ProviderRequest::Compact { thread }
                    });
                }
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
    pub(super) fn call_provider(
        &mut self,
        id: &ThreadId,
        action: Action,
        responder: Option<Responder>,
        request: impl FnOnce(ThreadId) -> ProviderRequest,
    ) {
        self.call_provider_of(id, None, action, responder, request);
    }

    /// Like [`Core::call_provider`], to `provider` instead of the thread's own.
    pub(super) fn call_provider_of(
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

    pub(super) fn call_finished(
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
                if matches!(action, Action::Input | Action::Compact)
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
            // A plugin turned off already ended its threads' sessions.
            Err(PluginError::Gone) => {
                let live = self.supervisor.engine(&id).is_ok_and(|engine| engine.session_live());
                let starting = self.sessions.get(&id).is_some_and(|session| session.starting);
                if live || starting {
                    self.session_failed(&id, "provider plugin is gone".into());
                }
            }
        }
        let now = now_ms();
        if matches!(action, Action::Interrupt | Action::Release | Action::Reset)
            && let Some(session) = self.sessions.get_mut(&id)
        {
            session.stopping = false;
        }
        match action {
            Action::Input
            | Action::Steer
            | Action::Resolve
            | Action::UpdateMcp
            | Action::Compact => {}
            Action::Interrupt => {
                // The backstop: a bridge that settled its own turn makes this a no-op.
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.force_settle(now);
                    self.commit(events);
                }
            }
            Action::Release | Action::Archive { .. } | Action::Reset => {
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let mut events = engine.force_settle(now);
                    events.extend(engine.session_released(Some("released".into()), now));
                    self.commit(events);
                }
            }
        }
        if matches!(action, Action::UpdateMcp)
            && let Some(session) = self.sessions.get_mut(&id)
        {
            session.mcp_updating = false;
        }
        // A change that waited for this call: after an interrupt the session takes
        // it, after a release the next session starts with it anyway.
        self.push_stale_mcp(&id);
        if matches!(action, Action::Release)
            && let Some(environment) =
                self.sessions.get_mut(&id).and_then(|session| session.prepare_after_release.take())
        {
            self.prepare_environment(&id, environment, None);
        }
        if let Action::Archive { force } = action {
            if let Some(responder) = responder {
                self.remove_environment(id, force, responder);
            }
            return;
        }
        if let Action::Reset = action {
            let result = self.record_reset(&id);
            if let Some(responder) = responder {
                responder.reply(result);
            }
            return;
        }
        if let Some(responder) = responder {
            // Whatever the provider said, the thread is settled now, which is what
            // the caller asked for.
            responder.reply(Ok(Empty {}));
        }
    }

    pub(super) fn deltas(&mut self, plugin: &str, id: ThreadId, deltas: Vec<Delta>) {
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

    pub(super) fn session_failed(&mut self, id: &ThreadId, message: String) {
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

    pub(super) fn plugin_crashed(&mut self, plugin: &str, reason: &str) {
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
}

/// The qcc MCP servers a thread's session gets: those on in the settings that the
/// thread does not keep off. The agent's own servers are its configuration's.
fn mcp_servers_for(
    settings: &BTreeMap<String, McpServerSettings>,
    disabled: &[String],
) -> Vec<McpServer> {
    settings
        .iter()
        .filter(|(name, server)| !server.disabled && !disabled.contains(name))
        .map(|(name, server)| McpServer { name: name.clone(), transport: server.transport.clone() })
        .collect()
}
