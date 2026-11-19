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
use qcc_domain::{IconColor, Project, ProjectIcon};

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
        /// An icon of the app's set by name (`rocket`, `gamepad-2`) or an emoji.
        #[arg(long)]
        icon: Option<String>,
        /// The color of a named icon: brand, red, orange, yellow, green, teal, blue,
        /// violet, pink or gray.
        #[arg(long, requires = "icon", value_parser = parse_color)]
        color: Option<IconColor>,
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
    /// Set the icon the app draws for a project: a name from the app's icon set
    /// (`rocket`, `gamepad-2`), an emoji, or `none` for the default.
    Icon {
        project: String,
        icon: String,
        /// The color of a named icon: brand, red, orange, yellow, green, teal, blue,
        /// violet, pink or gray.
        #[arg(long, value_parser = parse_color)]
        color: Option<IconColor>,
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
        icon: None,
    };
    let changed = match command {
        ProjectCommand::List => {
            let result = client.call::<ProjectList>(Empty {}).await?;
            if json {
                print_json(&result)?;
            } else {
                for project in &result.projects {
                    match &project.icon {
                        Some(icon) => {
                            out(format!("{}  {}  ({})", project.id, project.name, icon_text(icon)));
                        }
                        None => out(format!("{}  {}", project.id, project.name)),
                    }
                    for folder in &project.folders {
                        out(format!("  {folder}"));
                    }
                }
            }
            return Ok(ExitCode::SUCCESS);
        }
        ProjectCommand::Create { name, folders, icon, color } => {
            let folders = folders.into_iter().map(absolute).collect::<anyhow::Result<_>>()?;
            let icon = icon.map(|icon| icon_of(&icon, color)).transpose()?.flatten();
            client.call::<ProjectCreate>(ProjectCreateParams { name, folders, icon }).await?
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
        ProjectCommand::Icon { project, icon, color } => {
            let project = find_project(client, &project).await?;
            let params =
                ProjectUpdateParams { icon: Some(icon_of(&icon, color)?), ..update(&project) };
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

/// What `icon` on the command line means: `none` is no icon, a kebab-case word is
/// the name of an icon, anything else an emoji. The daemon checks the rest.
fn icon_of(icon: &str, color: Option<IconColor>) -> anyhow::Result<Option<ProjectIcon>> {
    let icon = icon.trim();
    let named = !icon.is_empty()
        && icon.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if icon == "none" {
        if color.is_some() {
            bail!("--color needs an icon");
        }
        return Ok(None);
    }
    if named {
        return Ok(Some(ProjectIcon::Lucide { name: icon.to_owned(), color }));
    }
    if color.is_some() {
        bail!("--color is for named icons; an emoji has its own colors");
    }
    Ok(Some(ProjectIcon::Emoji { emoji: icon.to_owned() }))
}

/// A palette color by the name the protocol gives it.
fn parse_color(name: &str) -> Result<IconColor, String> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).map_err(|_| {
        "not a color: brand, red, orange, yellow, green, teal, blue, violet, pink or gray".into()
    })
}

fn icon_text(icon: &ProjectIcon) -> String {
    match icon {
        ProjectIcon::Lucide { name, color: Some(color) } => {
            let color = serde_json::to_value(color).ok();
            format!("{name}, {}", color.as_ref().and_then(|c| c.as_str()).unwrap_or("?"))
        }
        ProjectIcon::Lucide { name, color: None } => name.clone(),
        ProjectIcon::Emoji { emoji } => emoji.clone(),
    }
}
