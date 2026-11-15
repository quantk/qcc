//! Provider sessions: starting them, calling providers and taking their deltas.

use qcc_domain::{Delta, EnvironmentState, EventPayload, McpServer, ProviderId, Role, ThreadId};
use qcc_plugin_host::{Outcome, PluginError, PluginRequest, ProviderRequest, SessionStart};
use qcc_protocol::methods::Empty;
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
    pub(super) fn start_session(&mut self, id: &ThreadId, prompt: Option<String>) {
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
        let (mcp_servers, mcp_disabled) =
            self.session_mcp(&thread.mcp_disabled, info.capabilities.mcp_toggle);
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
                mcp_servers,
                mcp_disabled,
            }),
        };
        self.sessions.entry(id.clone()).or_default().starting = true;
        let (plugin, commands, thread) = (plugin.clone(), self.commands.clone(), id.clone());
        tokio::spawn(async move {
            let result = plugin.call(request).await;
            let _ = commands.send(Command::SessionStartFinished { thread, resumed, result });
        });
    }

    /// The MCP servers a session gets, and those of them or of the agent's own
    /// configuration it keeps off.
    ///
    /// A provider that switches servers of a running session gets every server the
    /// user did not turn off, the thread's choice among them included, so that one
    /// can be switched on later without a new session. Another one is given only
    /// the servers the thread wants.
    fn session_mcp(&self, disabled: &[String], toggle: bool) -> (Vec<McpServer>, Vec<String>) {
        let servers: Vec<McpServer> = self
            .settings
            .mcp_servers
            .iter()
            .filter(|(name, server)| !server.disabled && (toggle || !disabled.contains(name)))
            .map(|(name, server)| McpServer {
                name: name.clone(),
                transport: server.transport.clone(),
            })
            .collect();
        // A qcc server the session does not get is not the agent's to turn off.
        let disabled = disabled
            .iter()
            .filter(|name| {
                !self.settings.mcp_servers.contains_key(*name)
                    || servers.iter().any(|server| &&server.name == name)
            })
            .cloned()
            .collect();
        (servers, disabled)
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
        // A change that came while the session started is not in it.
        if session.release_when_idle
            && let Some(next) = self.sessions.get_mut(&id)
        {
            next.release_when_idle = true;
        }
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
            Action::Input | Action::Steer | Action::Resolve | Action::SwitchMcp => {}
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
