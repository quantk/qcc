//! Requests that create threads and change their input, mode or agent.

use qcc_domain::{
    EnvironmentState, EventPayload, PermissionMode, ProviderId, Thread, ThreadEnvironment,
    ThreadEvent, ThreadId, ThreadState, UserInput,
};
use qcc_plugin_host::{EnvironmentRequest, Outcome, ProviderRequest};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    Delivery, Empty, InteractionResolveParams, MessageContext, SendMode, StopMode,
    ThreadCompactParams, ThreadDeliverParams, ThreadDeliverResult, ThreadEventsParams,
    ThreadEventsResult, ThreadIsolateParams, ThreadRenameParams, ThreadResetParams, ThreadResult,
    ThreadSendParams, ThreadSendResult, ThreadSetAgentParams, ThreadSetMcpParams,
    ThreadSetModeParams, ThreadSetParentParams, ThreadSetProjectParams, ThreadSpawnParams,
    ThreadStopParams, ThreadUnqueueParams, ThreadUnqueueResult,
};
use qcc_runtime::NewThread;

use super::{
    Action, Core, MAX_SPAWN_DEPTH, QUERY_TIMEOUT, Responder, Subscriber, call_environment,
    environment_error, now_ms, thread_not_found,
};

impl Core {
    pub(super) fn thread_spawn(
        &mut self,
        mut p: ThreadSpawnParams,
    ) -> Result<ThreadResult, RpcError> {
        let parent = match &p.parent {
            Some(parent) => {
                let thread =
                    self.supervisor.thread(parent).ok_or_else(|| thread_not_found(parent))?;
                if self.depth(parent) >= MAX_SPAWN_DEPTH {
                    return Err(RpcError::new(
                        codes::INVALID_PARAMS,
                        format!(
                            "thread {parent} is {MAX_SPAWN_DEPTH} levels below the thread the \
                             user started; agents may not delegate any deeper"
                        ),
                    ));
                }
                Some(thread.clone())
            }
            None => None,
        };
        let defaults = self.settings.providers.get(&p.provider).cloned().unwrap_or_default();
        // A child takes after its parent; what the user set for the agent fills in the rest.
        // A model name and an effort level mean something only to the provider they
        // were chosen for.
        let same_agent = parent.as_ref().filter(|parent| parent.provider == p.provider);
        if p.model.is_none() {
            p.model = same_agent.and_then(|parent| parent.model.clone()).or(defaults.model);
        }
        if p.effort.is_none() {
            p.effort = same_agent.and_then(|parent| parent.effort.clone()).or(defaults.effort);
        }
        p.permission_mode = match (&parent, p.permission_mode) {
            (_, Some(mode)) => Some(mode),
            (None, None) => defaults.permission_mode,
            (Some(parent), None) => {
                let wanted = parent.permission_mode.or(defaults.permission_mode);
                match wanted {
                    // The parent's mode, or the user's default for this agent, tightened
                    // to what the parent allows and then to what this agent can enforce.
                    Some(wanted) => {
                        let bound = wanted.min(ceiling(parent.permission_mode));
                        Some(bound.fit(&self.permission_modes(&p.provider)?).ok_or_else(|| {
                            RpcError::new(
                                codes::INVALID_PARAMS,
                                format!(
                                    "provider {} cannot run as strictly as its parent ({})",
                                    p.provider,
                                    bound.as_str()
                                ),
                            )
                        })?)
                    }
                    None => None,
                }
            }
        };
        self.check_mode(&p.provider, p.permission_mode)?;
        if let Some(parent) = &parent {
            check_ceiling(parent, p.permission_mode)?;
        }
        self.check_skills(p.skills_enabled.iter().chain(&p.skills_disabled))?;
        if let Some(by) = &p.started_by
            && self.supervisor.thread(by).is_none()
        {
            return Err(thread_not_found(by));
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
                    return Err(self.environment_missing(&id, codes::INVALID_PARAMS));
                }
                Some(ThreadEnvironment {
                    id,
                    source_dir: p.cwd.clone(),
                    base: p.base,
                    state: EnvironmentState::Pending,
                    carry_changes: false,
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
        let attachments = std::mem::take(&mut p.attachments);
        let attachments = self.store_attachments(&id, attachments)?;
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
                    effort: p.effort,
                    parent: p.parent,
                    project,
                    permission_mode: p.permission_mode,
                    skills_enabled: p.skills_enabled,
                    skills_disabled: p.skills_disabled,
                    started_by: p.started_by,
                    hidden: p.hidden,
                },
                now,
            )
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        let prompt = match (p.prompt, attachments) {
            (None, attachments) if attachments.is_empty() => None,
            (text, attachments) => {
                Some(UserInput { text: text.unwrap_or_default(), notes: Vec::new(), attachments })
            }
        };
        if let Some(prompt) = &prompt
            && let Ok(engine) = self.supervisor.engine(&id)
        {
            events.extend(engine.submit_input(prompt.clone(), now));
        }
        self.commit(events);
        self.start_session(&id, prompt);

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
        let (_, info) = self.provider_of(provider)?;
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

    /// The modes a provider can enforce.
    fn permission_modes(&self, provider: &ProviderId) -> Result<Vec<PermissionMode>, RpcError> {
        self.provider_of(provider).map(|(_, info)| info.capabilities.permission_modes.clone())
    }

    /// The thread that started `thread`, if it is still known.
    fn parent_of(&self, thread: &ThreadId) -> Option<Thread> {
        let parent = self.supervisor.thread(thread)?.parent.clone()?;
        self.supervisor.thread(&parent).cloned()
    }

    pub(super) fn thread_send(
        &mut self,
        p: ThreadSendParams,
    ) -> Result<ThreadSendResult, RpcError> {
        let ThreadSendParams { thread_id, text, delivery, context, attachments } = p;
        if text.trim().is_empty() && attachments.is_empty() {
            return Err(RpcError::new(codes::INVALID_PARAMS, "the message is empty"));
        }
        let notes = context.iter().map(|context| self.context_note(context)).collect();
        self.check_open(&thread_id)?;
        self.check_runnable(&thread_id)?;
        let attachments = self.store_attachments(&thread_id, attachments)?;
        let input = UserInput { text, notes, attachments };
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
            let input = self.with_pending_notes(&thread_id, input);
            let Ok(engine) = self.supervisor.engine(&thread_id) else {
                return Err(thread_not_found(&thread_id));
            };
            let events = engine.submit_input(input.clone(), now);
            self.commit(events);
            self.deliver(&thread_id, input);
            return Ok(ThreadSendResult { delivery: Delivery::Started, queue_id: None });
        }

        if delivery == SendMode::Steer
            && can_steer
            && !held
            && engine.turn_open()
            && engine.session_live()
        {
            let input = self.with_pending_notes(&thread_id, input);
            let Ok(engine) = self.supervisor.engine(&thread_id) else {
                return Err(thread_not_found(&thread_id));
            };
            let events = engine.steer_input(input.clone(), now);
            self.commit(events);
            self.call_provider(&thread_id, Action::Steer, None, |thread| ProviderRequest::Steer {
                thread,
                input,
            });
            return Ok(ThreadSendResult { delivery: Delivery::Steered, queue_id: None });
        }

        let events = engine.queue_input(input, now);
        let queue_id = events.iter().find_map(|event| match &event.payload {
            EventPayload::InputQueued { queue_id, .. } => Some(queue_id.clone()),
            _ => None,
        });
        self.commit(events);
        self.queued.insert(thread_id);
        Ok(ThreadSendResult { delivery: Delivery::Queued, queue_id })
    }

    pub(super) fn thread_isolate(
        &mut self,
        p: ThreadIsolateParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadIsolateParams { thread_id, environment, carry_changes } = p;
        let thread =
            self.supervisor.thread(&thread_id).ok_or_else(|| thread_not_found(&thread_id))?;
        if thread.environment.is_some() {
            return Err(environment_error(format!(
                "thread {thread_id} already works in an environment of its own"
            )));
        }
        if self.host.environment(&environment).is_none() {
            return Err(self.environment_missing(&environment, codes::INVALID_PARAMS));
        }
        let attached = ThreadEnvironment {
            id: environment,
            source_dir: thread.cwd.clone(),
            base: None,
            state: EnvironmentState::Pending,
            carry_changes,
        };
        let payload = EventPayload::EnvironmentAttached { environment: attached.clone() };
        let result = self.reconfigure(&thread_id, "where the thread works", |engine, now| {
            engine.record(payload, now)
        })?;
        self.isolating.insert(thread_id.clone());
        let releasing = self.sessions.get(&thread_id).is_some_and(|session| session.stopping);
        if releasing {
            self.sessions.entry(thread_id.clone()).or_default().prepare_after_release =
                Some(attached);
        } else {
            self.prepare_environment(&thread_id, attached, None);
        }
        let thread = self.supervisor.thread(&thread_id).cloned();
        Ok(thread.map(|thread| ThreadResult { thread }).unwrap_or(result))
    }

    /// `thread.deliver`: the environment plugin brings the work back, off the core task.
    pub(super) fn thread_deliver(&mut self, p: ThreadDeliverParams, responder: Responder) {
        let ThreadDeliverParams { thread_id, mode } = p;
        let busy = self.in_transition(&thread_id)
            || self.supervisor.engine(&thread_id).is_ok_and(|engine| engine.busy());
        if busy {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "a turn is running; let it finish before bringing its work back",
            )));
        }
        let (plugin, environment, cwd) = match self.ready_environment(&thread_id) {
            Ok(Some(found)) => found,
            Ok(None) => {
                return responder.reply::<Empty>(Err(environment_error(format!(
                    "thread {thread_id} works in its source directory already; there is \
                     nothing to bring back"
                ))));
            }
            Err(error) => return responder.reply::<Empty>(Err(error)),
        };
        let request = EnvironmentRequest::Deliver { thread: thread_id, cwd, mode };
        tokio::spawn(async move {
            let result = match call_environment(plugin, environment, request, QUERY_TIMEOUT).await {
                Ok(Outcome::EnvironmentDelivered(message)) => Ok(ThreadDeliverResult { message }),
                Ok(other) => Err(environment_error(format!("the environment answered {other:?}"))),
                Err(error) => Err(environment_error(error.to_string())),
            };
            responder.reply(result);
        });
    }

    pub(super) fn thread_rename(
        &mut self,
        p: ThreadRenameParams,
    ) -> Result<ThreadResult, RpcError> {
        let title: String = p.title.trim().chars().take(200).collect();
        if title.is_empty() {
            return Err(RpcError::new(codes::INVALID_PARAMS, "a thread needs a non-empty name"));
        }
        let engine =
            self.supervisor.engine(&p.thread_id).map_err(|_| thread_not_found(&p.thread_id))?;
        let events = engine.set_title(title, now_ms());
        self.commit(events);
        self.thread_result(&p.thread_id)
    }

    pub(super) fn thread_set_project(
        &mut self,
        p: ThreadSetProjectParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetProjectParams { thread_id, project } = p;
        if let Some(project) = &project {
            self.project(project)?;
        }
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let events = engine.set_project(project, now_ms());
        self.commit(events);
        self.thread_result(&thread_id)
    }

    pub(super) fn thread_set_parent(
        &mut self,
        p: ThreadSetParentParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetParentParams { thread_id, parent } = p;
        let thread = self
            .supervisor
            .thread(&thread_id)
            .cloned()
            .ok_or_else(|| thread_not_found(&thread_id))?;
        if let Some(parent_id) = &parent {
            let parent = self
                .supervisor
                .thread(parent_id)
                .cloned()
                .ok_or_else(|| thread_not_found(parent_id))?;
            // Walk up from the new parent: meeting the thread would close a loop.
            let mut current = Some(parent.id.clone());
            while let Some(id) = current {
                if id == thread_id {
                    return Err(RpcError::new(
                        codes::INVALID_PARAMS,
                        format!("thread {parent_id} is {thread_id} or was started under it"),
                    ));
                }
                current = self.supervisor.thread(&id).and_then(|t| t.parent.clone());
            }
            // Its own children are already held to its mode, so only it is checked.
            check_ceiling(&parent, thread.permission_mode)?;
        }
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let events = engine.set_parent(parent, now_ms());
        self.commit(events);
        self.thread_result(&thread_id)
    }

    pub(super) fn thread_unqueue(
        &mut self,
        p: ThreadUnqueueParams,
    ) -> Result<ThreadUnqueueResult, RpcError> {
        let ThreadUnqueueParams { thread_id, queue_id } = p;
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let Some((input, events)) = engine.unqueue(&queue_id, now_ms()) else {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("no message {queue_id:?} is queued; it may have been delivered already"),
            ));
        };
        if !engine.has_queued_input() {
            self.queued.remove(&thread_id);
        }
        self.commit(events);
        Ok(ThreadUnqueueResult { text: input.text, attachments: input.attachments })
    }

    /// The provider session is starting or stopping: input waits, and nothing else
    /// may start or stop it until the provider answers.
    pub(super) fn in_transition(&self, id: &ThreadId) -> bool {
        self.sessions.get(id).is_some_and(|session| session.starting || session.stopping)
    }

    /// An archived thread takes no input, and neither does one being archived.
    pub(super) fn check_open(&self, id: &ThreadId) -> Result<(), RpcError> {
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
    fn deliver(&mut self, id: &ThreadId, input: UserInput) {
        let live = self.supervisor.engine(id).is_ok_and(|engine| engine.session_live());
        if live {
            self.call_provider(id, Action::Input, None, |thread| ProviderRequest::Input {
                thread,
                input,
            });
        } else {
            self.start_session(id, Some(input));
        }
    }

    /// What qcc has to tell the thread's agent with its next message, taken off the
    /// list. Nothing when no session is attached: the next one is told at its start.
    fn take_notes(&mut self, id: &ThreadId) -> Vec<String> {
        let live = self.supervisor.engine(id).is_ok_and(|engine| engine.session_live());
        let Some(session) = self.sessions.get_mut(id) else { return Vec::new() };
        let notes = std::mem::take(&mut session.notes);
        if live { notes } else { Vec::new() }
    }

    fn with_pending_notes(&mut self, id: &ThreadId, mut input: UserInput) -> UserInput {
        input.notes.extend(self.take_notes(id));
        input
    }

    /// Something to tell the thread's agent with its next message.
    pub(super) fn add_note(&mut self, id: &ThreadId, note: String) {
        self.sessions.entry(id.clone()).or_default().notes.push(note);
    }

    /// The note a message's context becomes: a line the agent can act on.
    fn context_note(&self, context: &MessageContext) -> String {
        match context {
            MessageContext::Thread { thread_id } => {
                let Some(summary) = self.summary(thread_id) else {
                    return format!("The user is looking at qcc thread {thread_id}.");
                };
                let thread = summary.thread;
                let state = match summary.state {
                    ThreadState::Working => "working",
                    ThreadState::Waiting => "waiting for the user",
                    ThreadState::Failed => "failed",
                    ThreadState::Archived => "archived",
                    ThreadState::Idle => "idle",
                };
                let title = thread.title.as_deref().unwrap_or("untitled");
                format!(
                    "The user is looking at qcc thread {thread_id} ({title:?}, run by {}, {state}). \
                     `qcc thread show {thread_id}` reads it.",
                    thread.provider
                )
            }
            MessageContext::Comments { comment_ids } => self.comments_note(comment_ids),
        }
    }

    /// Release the sessions that wait for their turn to end to take a change.
    pub(super) fn release_idle(&mut self) {
        let waiting: Vec<ThreadId> = self
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.release_when_idle && !session.starting && !session.stopping
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in waiting {
            let Ok(engine) = self.supervisor.engine(&id) else { continue };
            if engine.busy() {
                continue;
            }
            let live = engine.session_live();
            let Some(session) = self.sessions.get_mut(&id) else { continue };
            session.release_when_idle = false;
            if live {
                session.stopping = true;
                self.call_provider(&id, Action::Release, None, |thread| ProviderRequest::Stop {
                    thread,
                });
            }
        }
    }

    /// Deliver queued messages of every thread that has become free. One message per
    /// thread: the next one waits for the turn this one starts.
    pub(super) fn drain_queues(&mut self) {
        if self.queued.is_empty() {
            return;
        }
        let now = now_ms();
        for id in self.queued.clone() {
            // Messages for an agent whose plugin is off wait until it is back on.
            if self.in_transition(&id) || self.check_runnable(&id).is_err() {
                continue;
            }
            let Ok(engine) = self.supervisor.engine(&id) else {
                self.queued.remove(&id);
                continue;
            };
            if engine.busy() {
                continue;
            }
            let notes = self.take_notes(&id);
            let Ok(engine) = self.supervisor.engine(&id) else { continue };
            let delivery = engine.deliver_queued(&notes, now);
            if !engine.has_queued_input() {
                self.queued.remove(&id);
            }
            match delivery {
                Some((input, events)) => {
                    self.commit(events);
                    self.deliver(&id, input);
                }
                // Not delivered after all: keep what was to go with it.
                None => {
                    for note in notes {
                        self.add_note(&id, note);
                    }
                }
            }
        }
    }

    pub(super) fn thread_set_mcp(
        &mut self,
        p: ThreadSetMcpParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetMcpParams { thread_id, server, enabled } = p;
        if server.trim().is_empty() {
            return Err(RpcError::new(codes::INVALID_PARAMS, "name the MCP server"));
        }
        // The agent's own servers are its configuration's business: qcc shows them and
        // switches only its own. Turning a name on is let through, so that one kept
        // off before can be cleared.
        if !enabled && !self.settings.mcp_servers.contains_key(&server) {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!(
                    "{server} is not an MCP server set up in qcc; servers of the agent's own \
                     configuration are switched in the agent"
                ),
            ));
        }
        let thread =
            self.supervisor.thread(&thread_id).ok_or_else(|| thread_not_found(&thread_id))?;
        let mut disabled = thread.mcp_disabled.clone();
        if enabled {
            disabled.retain(|name| name != &server);
        } else {
            disabled.push(server.clone());
        }
        self.check_open(&thread_id)?;
        let before = self.thread_mcp(&thread_id);
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        let events = engine.set_mcp_disabled(disabled, now_ms());
        self.commit(events);
        self.mcp_changed(&thread_id, &before);
        self.thread_result(&thread_id)
    }

    pub(super) fn thread_set_mode(
        &mut self,
        p: ThreadSetModeParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetModeParams { thread_id, permission_mode } = p;
        let provider = self
            .supervisor
            .thread(&thread_id)
            .map(|thread| thread.provider.clone())
            .ok_or_else(|| thread_not_found(&thread_id))?;
        self.check_mode(&provider, permission_mode)?;
        if let Some(parent) = self.parent_of(&thread_id) {
            check_ceiling(&parent, permission_mode)?;
        }
        self.reconfigure(&thread_id, "the permission mode", |engine, now| {
            engine.set_permission_mode(permission_mode, now)
        })
    }

    pub(super) fn thread_set_agent(
        &mut self,
        p: ThreadSetAgentParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadSetAgentParams { thread_id, provider, model, effort } = p;
        let supported = self.permission_modes(&provider)?;
        let model = model.filter(|model| !model.trim().is_empty());
        let effort = effort.filter(|effort| !effort.trim().is_empty());
        self.reconfigure(&thread_id, "the agent", |engine, now| {
            let mut events = Vec::new();
            // A mode the new agent cannot enforce would fail every start. The nearest
            // stricter one keeps the user's intent; dropping the mode would hand the
            // thread to the agent's own configuration, which may allow anything.
            if let Some(mode) = engine.thread().permission_mode
                && !supported.contains(&mode)
            {
                events.extend(engine.set_permission_mode(mode.fit(&supported), now));
            }
            events.extend(engine.set_agent(provider, model, effort, now));
            events
        })
    }

    /// `thread.reset`: a live session is released first, so that its end is part of
    /// the conversation it ends. Answered once the new conversation has begun.
    pub(super) fn thread_reset(&mut self, p: ThreadResetParams, responder: Responder) {
        let ThreadResetParams { thread_id } = p;
        if let Err(error) = self.check_open(&thread_id) {
            return responder.reply::<Empty>(Err(error));
        }
        let held = self.in_transition(&thread_id);
        let Ok(engine) = self.supervisor.engine(&thread_id) else {
            return responder.reply::<Empty>(Err(thread_not_found(&thread_id)));
        };
        if engine.busy() || held || engine.has_queued_input() {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "a turn is running; stop it before starting the conversation afresh",
            )));
        }
        if engine.session_live() {
            self.sessions.entry(thread_id.clone()).or_default().stopping = true;
            self.call_provider(&thread_id, Action::Reset, Some(responder), |thread| {
                ProviderRequest::Stop { thread }
            });
        } else {
            let result = self.record_reset(&thread_id);
            responder.reply(result);
        }
    }

    pub(super) fn record_reset(&mut self, thread_id: &ThreadId) -> Result<ThreadResult, RpcError> {
        let engine = self.supervisor.engine(thread_id).map_err(|_| thread_not_found(thread_id))?;
        let events = engine.record(EventPayload::ConversationReset, now_ms());
        self.commit(events);
        // What the old session was to hear means nothing to the next one.
        if let Some(session) = self.sessions.get_mut(thread_id) {
            session.notes.clear();
            session.compact_after_start = false;
        }
        self.thread_result(thread_id)
    }

    pub(super) fn thread_compact(
        &mut self,
        p: ThreadCompactParams,
    ) -> Result<ThreadResult, RpcError> {
        let ThreadCompactParams { thread_id } = p;
        self.check_open(&thread_id)?;
        let thread =
            self.supervisor.thread(&thread_id).ok_or_else(|| thread_not_found(&thread_id))?;
        let can =
            self.host.provider(&thread.provider).is_some_and(|(_, info)| info.capabilities.compact);
        if !can {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("provider {} cannot compact a session's context", thread.provider),
            ));
        }
        let resumable = thread.provider_session.is_some();
        let held = self.in_transition(&thread_id);
        let engine =
            self.supervisor.engine(&thread_id).map_err(|_| thread_not_found(&thread_id))?;
        if engine.busy() || held || engine.has_queued_input() {
            return Err(RpcError::new(
                codes::THREAD_BUSY,
                "a turn is running; compact the context once it ends",
            ));
        }
        let live = engine.session_live();
        if !live && !resumable {
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                "the thread has no conversation to compact yet",
            ));
        }
        engine.compact_requested();
        if live {
            self.call_provider(&thread_id, Action::Compact, None, |thread| {
                ProviderRequest::Compact { thread }
            });
        } else {
            // Resume the session first; it is compacted once it is up.
            self.start_session(&thread_id, None);
            self.sessions.entry(thread_id.clone()).or_default().compact_after_start = true;
        }
        self.thread_result(&thread_id)
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
        self.thread_result(thread_id)
    }

    pub(super) fn thread_stop(&mut self, p: ThreadStopParams, responder: Responder) {
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

    pub(super) fn thread_events(
        &mut self,
        connection: u64,
        p: ThreadEventsParams,
        responder: Responder,
    ) {
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

    pub(super) fn interaction_resolve(
        &mut self,
        p: InteractionResolveParams,
    ) -> Result<Empty, RpcError> {
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
}

/// The loosest mode a child of `parent` may run in. A parent without a mode runs
/// as its agent is configured, which qcc cannot see; only `ask` is sure to be no
/// looser than that.
fn ceiling(parent: Option<PermissionMode>) -> PermissionMode {
    parent.unwrap_or(PermissionMode::Ask)
}

/// A thread may not run looser than the thread that started it: an agent must not
/// gain through a child what its own mode does not allow it.
fn check_ceiling(parent: &Thread, mode: Option<PermissionMode>) -> Result<(), RpcError> {
    let allowed = match (parent.permission_mode, mode) {
        (_, Some(mode)) => mode <= ceiling(parent.permission_mode),
        // Without a mode the child runs as its agent is configured, which may allow
        // anything; that is only as safe as a parent that does the same.
        (parent_mode, None) => parent_mode.is_none(),
    };
    if allowed {
        return Ok(());
    }
    let parent_mode =
        parent.permission_mode.map_or("its agent's own configuration", |m| m.as_str());
    Err(RpcError::new(
        codes::INVALID_PARAMS,
        format!(
            "a thread may not run looser than the thread that started it; {} runs in {}, \
             so its children may use at most {}",
            parent.id,
            parent_mode,
            ceiling(parent.permission_mode).as_str()
        ),
    ))
}
