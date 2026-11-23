//! Thread environments: preparing, querying and removing them.

use qcc_domain::{
    ChangeScope, EnvironmentState, EventPayload, ThreadEnvironment, ThreadId, UserInput,
};
use qcc_plugin_host::{EnvironmentRequest, Outcome, PluginError, PluginHandle};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{ArchiveStep, Empty, ThreadChangesResult, ThreadDiffResult};

use super::{
    Command, Core, PREPARE_TIMEOUT, QUERY_TIMEOUT, REMOVE_TIMEOUT, Responder, call_environment,
    environment_error, now_ms, thread_not_found,
};

impl Core {
    /// Check what a request says about where an environment works. Only an
    /// environment that declared `custom_placement` is told a directory or a branch;
    /// the directory is an absolute path, and no other thread has its environment
    /// there. Returns the directory in its plain form.
    pub(super) fn check_placement(
        &self,
        environment: Option<&str>,
        dir: Option<&str>,
        branch: Option<&str>,
        reuse_branch: bool,
    ) -> Result<Option<String>, RpcError> {
        let invalid = |message: String| RpcError::new(codes::INVALID_PARAMS, message);
        if dir.is_none() && branch.is_none() && !reuse_branch {
            return Ok(None);
        }
        let Some(environment) = environment else {
            return Err(invalid(
                "`environment_dir` and `branch` mean something only to an environment".into(),
            ));
        };
        let placeable =
            self.host.environment(environment).is_some_and(|(_, info)| info.custom_placement);
        if !placeable {
            return Err(invalid(format!(
                "environment {environment:?} chooses its directory and its branch itself"
            )));
        }
        if branch.is_some_and(str::is_empty) {
            return Err(invalid("`branch` is empty".into()));
        }
        if reuse_branch && branch.is_none() {
            return Err(invalid("`reuse_branch` needs the `branch` to reuse".into()));
        }
        let Some(dir) = dir else { return Ok(None) };
        let path = std::path::Path::new(dir);
        let plain = path.components().all(|part| {
            matches!(part, std::path::Component::RootDir | std::path::Component::Normal(_))
        });
        if !path.is_absolute() || !plain {
            return Err(invalid(format!(
                "environment_dir {dir:?} is not an absolute path free of `.` and `..`"
            )));
        }
        // Rebuilt from its parts: no doubled or trailing slashes.
        let dir = path.components().collect::<std::path::PathBuf>().display().to_string();
        let taken = self.supervisor.threads().find(|thread| {
            !thread.archived
                && thread.environment.as_ref().is_some_and(|environment| {
                    environment.state != EnvironmentState::Removed
                        && environment.dir.as_deref() == Some(dir.as_str())
                })
        });
        if let Some(thread) = taken {
            return Err(invalid(format!(
                "thread {} already has its environment in {dir}; archive it or name another directory",
                thread.id
            )));
        }
        Ok(Some(dir))
    }

    /// Ask the thread's environment for its working directory. The session starts
    /// when the answer comes back as [`Command::EnvironmentPrepared`].
    pub(super) fn prepare_environment(
        &mut self,
        id: &ThreadId,
        environment: ThreadEnvironment,
        prompt: Option<UserInput>,
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
            let message =
                self.environment_missing(&environment.id, codes::ENVIRONMENT_ERROR).message;
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
            dir: environment.dir,
            branch: environment.branch,
            reuse_branch: environment.reuse_branch,
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
        prompt: Option<UserInput>,
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
        let (plugin, _) = self
            .host
            .environment(&environment.id)
            .ok_or_else(|| self.environment_missing(&environment.id, codes::ENVIRONMENT_ERROR))?;
        Ok(Some((plugin.clone(), environment.id.clone(), thread.cwd.clone())))
    }

    /// Who answers `thread.changes` and `thread.diff`: the plugin of the thread's
    /// environment, or for a thread working in a directory of its own, an
    /// environment that inspects directories. The flag says which.
    fn changes_source(
        &self,
        id: &ThreadId,
    ) -> Result<(PluginHandle, String, String, bool), RpcError> {
        if let Some((plugin, environment, cwd)) = self.ready_environment(id)? {
            return Ok((plugin, environment, cwd, false));
        }
        let thread = self.supervisor.thread(id).ok_or_else(|| thread_not_found(id))?;
        if thread.environment.is_some() {
            return Err(environment_error(format!("the environment of thread {id} is not ready")));
        }
        let (plugin, info) = self.host.directory_inspector().ok_or_else(|| {
            environment_error(format!(
                "thread {id} has no environment, and no plugin can tell the changes of its directory"
            ))
        })?;
        Ok((plugin.clone(), info.id.clone(), thread.cwd.clone(), true))
    }

    /// `thread.changes`, or with `diff` (the paths to diff, all when empty, and
    /// whether whole files) `thread.diff`, of the part of the work `scope` names:
    /// answered by an environment plugin, off the core task.
    pub(super) fn environment_query(
        &mut self,
        id: &ThreadId,
        scope: ChangeScope,
        diff: Option<(Vec<String>, bool)>,
        responder: Responder,
    ) {
        let (plugin, environment, cwd, inspect) = match self.changes_source(id) {
            Ok(found) => found,
            Err(error) => return responder.reply::<Empty>(Err(error)),
        };
        let thread = id.clone();
        let request = match diff {
            Some((paths, full)) => {
                EnvironmentRequest::Diff { thread, cwd, inspect, paths, scope, full }
            }
            None => EnvironmentRequest::Changes { thread, cwd, inspect, scope },
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
        let name = self.host.environment(&environment).map(|(_, info)| info.name.clone());
        let step =
            ArchiveStep::RemovingEnvironment { environment: name.unwrap_or(environment.clone()) };
        self.archive_moved(&id, step);
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
}
