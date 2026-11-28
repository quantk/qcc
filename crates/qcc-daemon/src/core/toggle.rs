//! Turning plugins on and off while the daemon runs.
//!
//! A plugin turned off is not running at all: it is unloaded, its processes are
//! killed and everything it contributed goes away. The journal does not depend on
//! plugins, so threads made with it stay readable; they take no messages until it
//! is back.

use qcc_domain::{ProviderId, ThreadId};
use qcc_plugin_host::{Contribution, Permissions, PluginHandle};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    PLUGINS_UPDATED, PluginDisableParams, PluginEnableParams, PluginInfo, PluginListResult,
    PluginPermissions, PluginResult, PluginsUpdatedNotification, Settings,
};

use super::{Core, notify, now_ms};
use crate::{plugins, settings};

/// How many busy threads a refusal names before it only counts the rest.
const NAMED_THREADS: usize = 5;

fn permissions_info(permissions: &Permissions) -> PluginPermissions {
    PluginPermissions {
        deltas: permissions.deltas,
        kv: permissions.kv,
        timers: permissions.timers,
        threads: permissions.threads,
        skills: permissions.skills,
        turn_events: permissions.turn_events,
        comments: permissions.comments,
        files: permissions.files,
        thread_events: permissions.thread_events,
        notifications: permissions.notifications,
        process: permissions.process.clone(),
        home_files: permissions.home_files.clone(),
    }
}

impl Core {
    pub(super) fn plugin_list(&self) -> PluginListResult {
        PluginListResult { plugins: self.plugin_infos() }
    }

    /// Every installed plugin, in the order they load.
    fn plugin_infos(&self) -> Vec<PluginInfo> {
        self.installed.iter().map(|installed| self.plugin_info(&installed.manifest.id)).collect()
    }

    fn plugin_info(&self, id: &str) -> PluginInfo {
        if let Some(plugin) = self.host.plugin(id) {
            return running_info(plugin);
        }
        let manifest =
            self.installed.iter().map(|installed| &installed.manifest).find(|m| m.id == id);
        let enabled = !self.settings.disabled_plugins.iter().any(|off| off == id);
        PluginInfo {
            id: id.to_owned(),
            name: manifest.and_then(|m| m.name.clone()),
            description: manifest.and_then(|m| m.description.clone()),
            version: manifest.map(|m| m.version.clone()).unwrap_or_default(),
            bundled: plugins::is_bundled(id),
            enabled,
            error: self.load_errors.get(id).cloned().filter(|_| enabled),
            example: manifest.is_some_and(|m| m.example),
            permissions: manifest
                .map(|m| permissions_info(&plugins::permissions(&self.config, m)))
                .unwrap_or_default(),
            providers: Vec::new(),
            environments: Vec::new(),
            commands: Vec::new(),
            skills: Vec::new(),
            settings: Vec::new(),
        }
    }

    pub(super) fn plugin_enable(
        &mut self,
        p: PluginEnableParams,
    ) -> Result<PluginResult, RpcError> {
        let PluginEnableParams { id } = p;
        // A plugin put into the directory since the daemon started can be turned on too.
        match plugins::installed(&self.config) {
            Ok(installed) => self.installed = installed,
            Err(error) => tracing::warn!(%error, "cannot read the plugin directory"),
        }
        let Some(installed) = self.installed.iter().find(|i| i.manifest.id == id).cloned() else {
            return Err(no_plugin(&id));
        };
        if self.host.plugin(&id).is_none() {
            match plugins::load(
                &mut self.host,
                &self.config,
                &installed,
                &self.commands,
                &self.programs,
            ) {
                Ok(_) => {
                    self.load_errors.remove(&id);
                }
                Err(error) => {
                    // Left as it was: turned off stays off, and the error is shown
                    // with a plugin that should be running.
                    self.load_errors.insert(id.clone(), error.to_string());
                    return Err(RpcError::new(
                        codes::PLUGIN_ERROR,
                        format!("plugin {id} cannot start: {error}"),
                    ));
                }
            }
        }
        if self.settings.disabled_plugins.contains(&id) {
            let mut next = self.settings.clone();
            next.disabled_plugins.retain(|off| off != &id);
            if let Err(error) = self.save_settings(next) {
                // Not saved, so not on: running it now would be undone by a restart.
                self.host.unload(&id);
                return Err(error);
            }
        }
        tracing::info!(plugin = id, "plugin turned on");
        self.post_plugin_settings(&id);
        self.plugins_changed();
        Ok(PluginResult { plugin: self.plugin_info(&id) })
    }

    pub(super) fn plugin_disable(
        &mut self,
        p: PluginDisableParams,
    ) -> Result<PluginResult, RpcError> {
        let PluginDisableParams { id, force } = p;
        let installed = self.installed.iter().any(|i| i.manifest.id == id);
        if !installed && self.host.plugin(&id).is_none() {
            return Err(no_plugin(&id));
        }
        let owned = self.host.plugin(&id).map(plugins::Owned::of).unwrap_or_default();
        // Threads its agents run, and threads in its environments, whose agents may
        // be another plugin's: those are in its hands only while it prepares or
        // removes their directories.
        let (runs, houses) = self.threads_using(&owned);
        let mut busy: Vec<&ThreadId> = Vec::new();
        for thread in &runs {
            let working = self.supervisor.engine(thread).is_ok_and(|engine| engine.busy());
            if working || self.in_transition(thread) || self.archiving.contains(thread) {
                busy.push(thread);
            }
        }
        busy.extend(houses.iter().filter(|thread| {
            self.archiving.contains(*thread)
                || (self.in_transition(thread) && !self.environment_ready(thread))
        }));
        if !busy.is_empty() && !force {
            let mut names: Vec<String> =
                busy.iter().take(NAMED_THREADS).map(ToString::to_string).collect();
            if busy.len() > NAMED_THREADS {
                names.push(format!("{} more", busy.len() - NAMED_THREADS));
            }
            return Err(RpcError::new(
                codes::THREAD_BUSY,
                format!(
                    "plugin {id} is in use by threads that are working: {}; stop them first, \
                     or turn it off anyway with force",
                    names.join(", ")
                ),
            ));
        }

        if !self.settings.disabled_plugins.contains(&id) {
            let mut next = self.settings.clone();
            next.disabled_plugins.push(id.clone());
            next.disabled_plugins.sort();
            self.save_settings(next)?;
        }
        if self.host.plugin(&id).is_some() {
            self.close_sessions(&id, &runs);
            self.watches.retain(|watch| watch.plugin != id);
            for provider in &owned.providers {
                self.models.remove(provider);
                self.agent_skills.retain(|(owner, _), _| owner != provider);
                self.agent_commands.retain(|(owner, _), _| owner != provider);
            }
            if let Some(plugin) = self.host.unload(&id) {
                // The last handle waits for the plugin's thread to end, which takes
                // as long as the request it is handling: not on the core task.
                tokio::task::spawn_blocking(move || drop(plugin));
            }
            tracing::info!(plugin = id, "plugin turned off");
        }
        self.load_errors.remove(&id);
        self.plugins_changed();
        Ok(PluginResult { plugin: self.plugin_info(&id) })
    }

    /// The threads run by the plugin's agents, and the other threads that work in
    /// one of its environments.
    fn threads_using(&self, owned: &plugins::Owned) -> (Vec<ThreadId>, Vec<ThreadId>) {
        let mut runs = Vec::new();
        let mut houses = Vec::new();
        for thread in self.supervisor.threads() {
            if owned.providers.contains(&thread.provider) {
                runs.push(thread.id.clone());
            } else if thread
                .environment
                .as_ref()
                .is_some_and(|env| owned.environments.contains(&env.id))
            {
                houses.push(thread.id.clone());
            }
        }
        (runs, houses)
    }

    fn environment_ready(&self, id: &ThreadId) -> bool {
        self.supervisor
            .thread(id)
            .and_then(|thread| thread.environment.as_ref())
            .is_some_and(|environment| environment.state == qcc_domain::EnvironmentState::Ready)
    }

    /// End what the plugin is doing for these threads, as a daemon shutdown would: a
    /// turn still open is cut short, a session is recorded as ended. Its answers to
    /// calls on their way are not waited for: the plugin is about to stop.
    fn close_sessions(&mut self, plugin: &str, threads: &[ThreadId]) {
        let reason = format!("plugin {plugin} was turned off");
        let now = now_ms();
        for id in threads {
            let session = self.sessions.remove(id);
            self.isolating.remove(id);
            let Ok(engine) = self.supervisor.engine(id) else { continue };
            if session.is_some_and(|session| session.starting) {
                // No turn will open for the message that was starting the session.
                engine.input_rejected();
            }
            let mut events = engine.force_settle(now);
            events.extend(engine.session_released(Some(reason.clone()), now));
            self.commit(events);
        }
    }

    /// Save new settings and take them as the daemon's.
    fn save_settings(&mut self, next: Settings) -> Result<(), RpcError> {
        settings::save(&self.config.settings(), &next).map_err(|error| {
            RpcError::new(codes::INTERNAL_ERROR, format!("cannot save the settings: {error}"))
        })?;
        self.settings = next;
        Ok(())
    }

    /// After a plugin started or stopped: offer the skills of the plugins running
    /// now, and tell the connections that watch threads.
    fn plugins_changed(&mut self) {
        self.skills = plugins::write_skills(&self.config, &self.host);
        let plugins = self.plugin_infos();
        self.watchers.retain(|watcher| {
            let subscription = watcher.subscription;
            let params = PluginsUpdatedNotification { subscription, plugins: plugins.clone() };
            notify(&watcher.out, PLUGINS_UPDATED, params)
        });
    }

    /// The plugin turned off that contributed what `owns` looks for, as far as known.
    fn disabled_owner(&self, owns: impl Fn(&plugins::Owned) -> bool) -> Option<&str> {
        self.settings
            .disabled_plugins
            .iter()
            .find(|id| plugins::owned(&self.config, id).is_some_and(|owned| owns(&owned)))
            .map(String::as_str)
    }

    /// Why no plugin provides an agent: it may be one the user turned off.
    pub(super) fn provider_missing(&self, provider: &ProviderId) -> RpcError {
        match self.disabled_owner(|owned| owned.providers.contains(provider)) {
            Some(plugin) => RpcError::new(
                codes::PLUGIN_DISABLED,
                format!(
                    "agent {:?} belongs to plugin {plugin}, which is turned off; \
                     `qcc plugin enable {plugin}` turns it back on",
                    provider.as_str()
                ),
            ),
            None => RpcError::new(
                codes::PROVIDER_NOT_FOUND,
                format!("no plugin provides {:?}", provider.as_str()),
            ),
        }
    }

    /// Why no plugin provides an environment: it may be one the user turned off.
    /// `code` is the error otherwise.
    pub(super) fn environment_missing(&self, environment: &str, code: i32) -> RpcError {
        match self.disabled_owner(|owned| owned.environments.iter().any(|id| id == environment)) {
            Some(plugin) => RpcError::new(
                codes::PLUGIN_DISABLED,
                format!(
                    "environment {environment:?} belongs to plugin {plugin}, which is turned off; \
                     `qcc plugin enable {plugin}` turns it back on"
                ),
            ),
            None => RpcError::new(code, format!("no plugin provides environment {environment:?}")),
        }
    }

    /// A thread can take a message only while the plugins it needs run: its agent's,
    /// and the one that has to prepare its environment.
    pub(super) fn check_runnable(&self, id: &ThreadId) -> Result<(), RpcError> {
        let Some(thread) = self.supervisor.thread(id) else { return Ok(()) };
        self.provider_of(&thread.provider)?;
        if let Some(environment) = &thread.environment
            && environment.state != qcc_domain::EnvironmentState::Ready
            && self.host.environment(&environment.id).is_none()
        {
            return Err(self.environment_missing(&environment.id, codes::ENVIRONMENT_ERROR));
        }
        Ok(())
    }
}

/// What a running plugin says about itself.
fn running_info(plugin: &PluginHandle) -> PluginInfo {
    let permissions = plugin.permissions();
    let manifest = plugin.manifest();
    let mut info = PluginInfo {
        id: plugin.id().to_owned(),
        name: manifest.name.clone(),
        description: manifest.description.clone(),
        version: manifest.version.clone(),
        bundled: plugins::is_bundled(plugin.id()),
        enabled: true,
        error: None,
        example: manifest.example,
        permissions: permissions_info(permissions),
        providers: Vec::new(),
        environments: Vec::new(),
        commands: Vec::new(),
        skills: Vec::new(),
        settings: Vec::new(),
    };
    for contribution in plugin.contributions() {
        match contribution {
            Contribution::Provider(provider) => info.providers.push(provider.clone()),
            Contribution::Environment(environment) => {
                info.environments.push(environment.clone());
            }
            Contribution::Command(command) => info.commands.push(command.clone()),
            // Listed only if the plugin may offer it.
            Contribution::Skill(skill) if permissions.skills => {
                info.skills.push(skill.name.clone());
            }
            Contribution::Setting(field) => info.settings.push(field.clone()),
            Contribution::Skill(_) | Contribution::UiView(_) | Contribution::UiItem(_) => {}
        }
    }
    info
}

fn no_plugin(id: &str) -> RpcError {
    RpcError::new(
        codes::PLUGIN_ERROR,
        format!("no plugin {id:?} is installed; `qcc plugin list` lists them"),
    )
}
