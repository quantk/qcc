//! Projects and daemon settings.

use qcc_domain::{McpServer, Project, ProjectId, SettingField, ThreadId};
use qcc_plugin_host::{Contribution, PluginHandle, PluginRequest};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    Empty, PROJECTS_UPDATED, ProgramStatus, ProjectCreateParams, ProjectDeleteParams,
    ProjectResult, ProjectUpdateParams, ProjectsUpdatedNotification, Settings, SettingsResult,
    SettingsSetParams,
};

use super::{Core, notify, now_ms};
use crate::{projects, settings};

impl Core {
    pub(super) fn settings_set(
        &mut self,
        p: SettingsSetParams,
    ) -> Result<SettingsResult, RpcError> {
        let SettingsSetParams { settings: mut next } = p;
        // Turning plugins on and off starts and stops them; that takes its own methods.
        next.disabled_plugins = self.settings.disabled_plugins.clone();
        settings::check(&next).map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        self.check_plugin_settings(&next)?;
        let before = self.settings.plugins.clone();
        // The qcc MCP servers of every thread, to bring running sessions along with
        // a change of them.
        let mcp_before: Vec<(ThreadId, Vec<McpServer>)> = self
            .supervisor
            .threads()
            .filter(|thread| !thread.archived)
            .map(|thread| (thread.id.clone(), self.thread_mcp(&thread.id)))
            .collect();
        settings::save(&self.config.settings(), &next).map_err(|error| {
            RpcError::new(codes::INTERNAL_ERROR, format!("cannot save the settings: {error}"))
        })?;
        self.programs.set(next.programs.iter().map(|(name, path)| (name.clone(), path.into())));
        self.settings = next;
        // Another program may offer other models.
        self.models.clear();
        let changed: Vec<String> = self
            .host
            .plugins()
            .iter()
            .map(|plugin| plugin.id().to_owned())
            .filter(|id| before.get(id) != self.settings.plugins.get(id))
            .collect();
        for plugin in changed {
            self.post_plugin_settings(&plugin);
        }
        for (id, servers) in mcp_before {
            self.mcp_changed(&id, &servers);
        }
        Ok(self.settings_result())
    }

    /// The settings of a plugin that is loaded must be its own and fit them. Those
    /// of a plugin not loaded now are kept as they are, for when it is back.
    fn check_plugin_settings(&self, settings: &Settings) -> Result<(), RpcError> {
        let providers: Vec<String> =
            self.host.providers().map(|(_, info)| info.id.to_string()).collect();
        let providers: Vec<&str> = providers.iter().map(String::as_str).collect();
        for (id, values) in &settings.plugins {
            let Some(plugin) = self.host.plugin(id) else {
                continue;
            };
            let fields = setting_fields(plugin);
            for (name, value) in values {
                let field = fields.iter().find(|field| &field.id == name).ok_or_else(|| {
                    RpcError::new(
                        codes::INVALID_PARAMS,
                        format!("plugin {id} has no setting {name:?}"),
                    )
                })?;
                if let Some(problem) = field.problem(value, &providers) {
                    return Err(RpcError::new(codes::INVALID_PARAMS, format!("{id}: {problem}")));
                }
            }
        }
        Ok(())
    }

    /// Tell a plugin its settings, defaults filled in. Nothing for a plugin without.
    pub(super) fn post_plugin_settings(&self, id: &str) {
        let Some(plugin) = self.host.plugin(id) else {
            return;
        };
        let fields = setting_fields(plugin);
        if fields.is_empty() {
            return;
        }
        let set = self.settings.plugins.get(id);
        let values = fields
            .iter()
            .filter_map(|field| {
                let value = set.and_then(|set| set.get(&field.id)).or(field.default.as_ref())?;
                Some((field.id.clone(), value.clone()))
            })
            .collect();
        if plugin.post(PluginRequest::Settings(values)).is_err() {
            tracing::warn!(plugin = id, "cannot tell the plugin its settings: it is gone");
        }
    }

    pub(super) fn settings_result(&self) -> SettingsResult {
        let mut programs: Vec<ProgramStatus> = Vec::new();
        for plugin in self.host.plugins() {
            for name in &plugin.permissions().process {
                match programs.iter_mut().find(|program| &program.name == name) {
                    Some(program) => program.plugins.push(plugin.id().to_owned()),
                    None => {
                        let (path, error) =
                            match qcc_plugin_host::resolve_program(name, &self.programs) {
                                Ok(path) => (Some(path.display().to_string()), None),
                                Err(error) => (None, Some(error)),
                            };
                        programs.push(ProgramStatus {
                            name: name.clone(),
                            plugins: vec![plugin.id().to_owned()],
                            path,
                            error,
                        });
                    }
                }
            }
        }
        programs.sort_by(|a, b| a.name.cmp(&b.name));
        SettingsResult { settings: self.settings.clone(), programs }
    }

    pub(super) fn project(&self, id: &ProjectId) -> Result<&Project, RpcError> {
        self.projects.iter().find(|project| &project.id == id).ok_or_else(|| {
            RpcError::new(codes::INVALID_PARAMS, format!("project {id} does not exist"))
        })
    }

    pub(super) fn project_create(
        &mut self,
        p: ProjectCreateParams,
    ) -> Result<ProjectResult, RpcError> {
        let (name, folders) = projects::check(&p.name, &p.folders)
            .map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        let project = Project {
            id: ProjectId(format!("prj_{}", uuid::Uuid::now_v7().simple())),
            name,
            folders,
            created_at: now_ms(),
        };
        self.save_project(project.clone())?;
        Ok(ProjectResult { project })
    }

    pub(super) fn project_update(
        &mut self,
        p: ProjectUpdateParams,
    ) -> Result<ProjectResult, RpcError> {
        let current = self.project(&p.id)?.clone();
        let (name, folders) = projects::check(
            p.name.as_deref().unwrap_or(&current.name),
            p.folders.as_deref().unwrap_or(&current.folders),
        )
        .map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        let project = Project { name, folders, ..current };
        self.save_project(project.clone())?;
        Ok(ProjectResult { project })
    }

    pub(super) fn project_delete(&mut self, p: ProjectDeleteParams) -> Result<Empty, RpcError> {
        self.project(&p.id)?;
        self.store
            .delete_project(&p.id)
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        self.projects.retain(|project| project.id != p.id);
        // Its threads stay, outside any project.
        let members: Vec<ThreadId> = self
            .supervisor
            .threads()
            .filter(|thread| thread.project.as_ref() == Some(&p.id))
            .map(|thread| thread.id.clone())
            .collect();
        let now = now_ms();
        for id in members {
            if let Ok(engine) = self.supervisor.engine(&id) {
                let events = engine.set_project(None, now);
                self.commit(events);
            }
        }
        self.projects_changed();
        Ok(Empty {})
    }

    /// Store a new or changed project, then let threads outside any project that
    /// work in one of its folders join it.
    fn save_project(&mut self, project: Project) -> Result<(), RpcError> {
        self.store
            .save_project(&project)
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        match self.projects.iter_mut().find(|known| known.id == project.id) {
            Some(known) => *known = project.clone(),
            None => self.projects.push(project.clone()),
        }
        let joining: Vec<ThreadId> = self
            .supervisor
            .threads()
            .filter(|thread| thread.project.is_none() && !thread.archived)
            .filter(|thread| {
                let dir = thread.environment.as_ref().map_or(&thread.cwd, |env| &env.source_dir);
                qcc_domain::project_of(&self.projects, dir).map(|found| &found.id)
                    == Some(&project.id)
            })
            .map(|thread| thread.id.clone())
            .collect();
        let now = now_ms();
        for id in joining {
            if let Ok(engine) = self.supervisor.engine(&id) {
                let events = engine.set_project(Some(project.id.clone()), now);
                self.commit(events);
            }
        }
        self.projects_changed();
        Ok(())
    }

    /// Tell every connection that watches threads what the projects are now.
    fn projects_changed(&mut self) {
        let projects = self.projects.clone();
        self.watchers.retain(|watcher| {
            let subscription = watcher.subscription;
            let params = ProjectsUpdatedNotification { subscription, projects: projects.clone() };
            notify(&watcher.out, PROJECTS_UPDATED, params)
        });
    }
}

/// The settings a plugin declared.
fn setting_fields(plugin: &PluginHandle) -> Vec<SettingField> {
    plugin
        .contributions()
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::Setting(field) => Some(field.clone()),
            _ => None,
        })
        .collect()
}
