//! Projects and daemon settings.

use qcc_domain::{Project, ProjectId, ThreadId};
use qcc_protocol::jsonrpc::{Request, RpcError, codes};
use qcc_protocol::methods::{
    Empty, PROJECTS_UPDATED, ProgramStatus, ProjectCreateParams, ProjectDeleteParams,
    ProjectResult, ProjectUpdateParams, ProjectsUpdatedNotification, SettingsResult,
    SettingsSetParams,
};

use super::{Core, now_ms};
use crate::{projects, settings};

impl Core {
    pub(super) fn settings_set(
        &mut self,
        p: SettingsSetParams,
    ) -> Result<SettingsResult, RpcError> {
        let SettingsSetParams { settings: next } = p;
        settings::check(&next).map_err(|message| RpcError::new(codes::INVALID_PARAMS, message))?;
        settings::save(&self.config.settings(), &next).map_err(|error| {
            RpcError::new(codes::INTERNAL_ERROR, format!("cannot save the settings: {error}"))
        })?;
        self.programs.set(next.programs.iter().map(|(name, path)| (name.clone(), path.into())));
        self.settings = next;
        // Another program may offer other models.
        self.models.clear();
        Ok(self.settings_result())
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
            let notification = Request::notification(
                PROJECTS_UPDATED,
                serde_json::to_value(ProjectsUpdatedNotification {
                    subscription: watcher.subscription,
                    projects: projects.clone(),
                })
                .expect("projects serialize"),
            );
            let line = serde_json::to_string(&notification).expect("notifications serialize");
            watcher.out.send(line).is_ok()
        });
    }
}
