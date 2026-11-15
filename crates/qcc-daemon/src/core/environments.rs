//! Thread environments: preparing, querying and removing them, and archiving threads.

use qcc_domain::{EnvironmentState, EventPayload, ThreadEnvironment, ThreadId};
use qcc_plugin_host::{EnvironmentRequest, Outcome, PluginError, PluginHandle, ProviderRequest};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    Empty, ThreadArchiveParams, ThreadChangesResult, ThreadDiffResult, ThreadResult,
};

use super::{
    Action, Command, Core, PREPARE_TIMEOUT, QUERY_TIMEOUT, REMOVE_TIMEOUT, Responder,
    call_environment, environment_error, now_ms, thread_not_found,
};

impl Core {
    /// Ask the thread's environment for its working directory. The session starts
    /// when the answer comes back as [`Command::EnvironmentPrepared`].
    pub(super) fn prepare_environment(
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
            carry_changes: environment.carry_changes,
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

    pub(super) fn environment_prepared(
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
                self.isolating.remove(&id);
                if let Ok(engine) = self.supervisor.engine(&id) {
                    let events = engine.record(EventPayload::EnvironmentReady { cwd }, now_ms());
                    self.commit(events);
                }
                // A thread that moved in waits for its next message: that is what
                // the new session is handed the conversation with.
                let moved = self.supervisor.thread(&id).is_some_and(|thread| thread.handover);
                if prompt.is_some() || !moved {
                    self.start_session(&id, prompt);
                }
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
        if self.isolating.remove(id) {
            if let Ok(engine) = self.supervisor.engine(id) {
                let events = engine.record(EventPayload::EnvironmentDetached { message }, now_ms());
                self.commit(events);
            }
            return;
        }
        if let Ok(engine) = self.supervisor.engine(id) {
            let failed = EventPayload::EnvironmentFailed { message: message.clone() };
            let events = engine.record(failed, now_ms());
            self.commit(events);
        }
        self.session_failed(id, format!("cannot prepare the working directory: {message}"));
    }

    /// The environment of a thread that has one ready, and the plugin behind it.
    pub(super) fn ready_environment(
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
    pub(super) fn environment_query(&mut self, id: &ThreadId, diff: bool, responder: Responder) {
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

    /// Release the session, then remove the environment, then mark the thread.
    pub(super) fn thread_archive(&mut self, p: ThreadArchiveParams, responder: Responder) {
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

    pub(super) fn remove_environment(&mut self, id: ThreadId, force: bool, responder: Responder) {
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

    pub(super) fn environment_removed(
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
}
