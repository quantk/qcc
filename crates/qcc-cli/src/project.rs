//! `qcc project`: projects and their folders.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};
use clap::Subcommand;
use qcc_client::Client;
use qcc_client::protocol::methods::{
    Empty, ProjectCreate, ProjectCreateParams, ProjectDelete, ProjectDeleteParams, ProjectList,
    ProjectListResult, ProjectUpdate, ProjectUpdateParams,
};
use qcc_domain::Project;

use crate::{absolute, out, print_json};

#[derive(Subcommand)]
pub(crate) enum ProjectCommand {
    /// List projects and their folders.
    List,
    /// Make a project. Threads outside any project that work in its folders join it.
    Create {
        name: String,
        /// A repository or folder of the project; repeat for several.
        #[arg(long = "folder", required = true)]
        folders: Vec<PathBuf>,
    },
    /// Add a folder to a project.
    AddFolder {
        project: String,
        folder: PathBuf,
    },
    /// Take a folder out of a project.
    RemoveFolder {
        project: String,
        folder: PathBuf,
    },
    Rename {
        project: String,
        name: String,
    },
    /// Delete a project. Its threads stay, outside any project.
    Delete {
        project: String,
    },
}

pub(crate) async fn run(
    client: &Client,
    command: ProjectCommand,
    json: bool,
) -> anyhow::Result<ExitCode> {
    let update = |project: &Project| ProjectUpdateParams {
        id: project.id.clone(),
        name: None,
        folders: None,
    };
    let changed = match command {
        ProjectCommand::List => {
            let result = client.call::<ProjectList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for project in &result.projects {
                    out(format!("{}  {}", project.id, project.name));
                    for folder in &project.folders {
                        out(format!("  {folder}"));
                    }
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        ProjectCommand::Create { name, folders } => {
            let folders = folders.into_iter().map(absolute).collect::<anyhow::Result<_>>()?;
            client.call::<ProjectCreate>(ProjectCreateParams { name, folders }).await?
        }
        ProjectCommand::AddFolder { project, folder } => {
            let project = find_project(client, &project).await?;
            let mut folders = project.folders.clone();
            folders.push(absolute(folder)?);
            let params = ProjectUpdateParams { folders: Some(folders), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::RemoveFolder { project, folder } => {
            let project = find_project(client, &project).await?;
            let folder = absolute(folder)?;
            let folders: Vec<String> =
                project.folders.iter().filter(|known| **known != folder).cloned().collect();
            if folders.len() == project.folders.len() {
                bail!("{folder} is not a folder of {}", project.name);
            }
            let params = ProjectUpdateParams { folders: Some(folders), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::Rename { project, name } => {
            let project = find_project(client, &project).await?;
            let params = ProjectUpdateParams { name: Some(name), ..update(&project) };
            client.call::<ProjectUpdate>(params).await?
        }
        ProjectCommand::Delete { project } => {
            let project = find_project(client, &project).await?;
            client.call::<ProjectDelete>(ProjectDeleteParams { id: project.id }).await?;
            return Ok(ExitCode::SUCCESS);
        }
    };
    if json {
        print_json(&changed)?;
    } else {
        out(format!("{}  {}", changed.project.id, changed.project.name));
    }
    Ok(ExitCode::SUCCESS)
}

/// A project by id or by exact name.
pub(crate) async fn find_project(client: &Client, wanted: &str) -> anyhow::Result<Project> {
    let ProjectListResult { projects } = client.call::<ProjectList>(Empty {}).await?;
    let mut matches = projects.into_iter().filter(|p| p.id.as_str() == wanted || p.name == wanted);
    let found =
        matches.next().with_context(|| format!("no project {wanted:?}; see `qcc project list`"))?;
    if matches.next().is_some() {
        bail!("several projects are named {wanted:?}; use the id");
    }
    Ok(found)
}
