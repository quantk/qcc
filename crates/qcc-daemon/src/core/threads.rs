//! Requests that create threads and change their input, mode or agent.

use qcc_domain::{
    EnvironmentState, EventPayload, PermissionMode, ProviderId, ThreadEnvironment, ThreadEvent,
    ThreadId,
};
use qcc_plugin_host::{EnvironmentRequest, Outcome, ProviderRequest};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    Delivery, Empty, InteractionResolveParams, SendMode, StopMode, ThreadDeliverParams,
    ThreadDeliverResult, ThreadEventsParams, ThreadEventsResult, ThreadIsolateParams,
    ThreadRenameParams, ThreadResult, ThreadSendParams, ThreadSendResult, ThreadSetAgentParams,
    ThreadSetModeParams, ThreadSetProjectParams, ThreadSpawnParams, ThreadStopParams,
    ThreadUnqueueParams, ThreadUnqueueResult,
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

    pub(super) fn thread_send(
        &mut self,
        p: ThreadSendParams,
    ) -> Result<ThreadSendResult, RpcError> {
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
            return Err(RpcError::new(
                codes::INVALID_PARAMS,
                format!("no plugin provides environment {environment:?}"),
            ));
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
        let thread = self.supervisor.thread(&p.thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&p.thread_id))
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
        let thread = self.supervisor.thread(&thread_id).cloned();
        thread.map(|thread| ThreadResult { thread }).ok_or_else(|| thread_not_found(&thread_id))
    }

    pub(super) fn thread_unqueue(
        &mut self,
        p: ThreadUnqueueParams,
    ) -> Result<ThreadUnqueueResult, RpcError> {
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
    pub(super) fn in_transition(&self, id: &ThreadId) -> bool {
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
    pub(super) fn drain_queues(&mut self) {
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
        self.reconfigure(&thread_id, "the permission mode", |engine, now| {
            engine.set_permission_mode(permission_mode, now)
        })
    }

    pub(super) fn thread_set_agent(
        &mut self,
        p: ThreadSetAgentParams,
    ) -> Result<ThreadResult, RpcError> {
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
