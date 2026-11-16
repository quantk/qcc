//! Projects: named sets of folders that threads are grouped by.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ids::ProjectId;

/// What the user works on: one or several repositories or folders under a name.
/// A thread belongs to at most one project and works in one of its folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    /// Absolute paths, in the order the user gave them.
    pub folders: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
}

impl Project {
    /// How specific the project is about `dir`: the length of the longest folder
    /// that holds it. A project for a subdirectory wins over one for its parent.
    fn depth_of(&self, dir: &str) -> Option<usize> {
        self.folders
            .iter()
            .filter(|folder| Path::new(dir).starts_with(folder))
            .map(|folder| Path::new(folder).components().count())
            .max()
    }
}

/// The project a directory belongs to: the one with the most specific folder.
pub fn project_of<'a>(
    projects: impl IntoIterator<Item = &'a Project>,
    dir: &str,
) -> Option<&'a Project> {
    projects
        .into_iter()
        .filter_map(|project| project.depth_of(dir).map(|depth| (depth, project)))
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, project)| project)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, folders: &[&str]) -> Project {
        Project {
            id: ProjectId::new(id),
            name: id.into(),
            folders: folders.iter().map(|f| (*f).to_owned()).collect(),
            created_at: 0,
        }
    }

    #[test]
    fn a_directory_belongs_to_the_most_specific_project() {
        let projects = [project("all", &["/src"]), project("app", &["/src/app", "/srv/api"])];
        assert_eq!(project_of(&projects, "/src/app/ui").map(|p| p.id.as_str()), Some("app"));
        assert_eq!(project_of(&projects, "/srv/api").map(|p| p.id.as_str()), Some("app"));
        assert_eq!(project_of(&projects, "/src/lib").map(|p| p.id.as_str()), Some("all"));
        assert_eq!(project_of(&projects, "/home"), None);
        // Whole components only: /src/application is not inside /src/app.
        assert_eq!(projects[1].depth_of("/src/application"), None);
    }
}
