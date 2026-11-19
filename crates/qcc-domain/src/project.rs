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
    /// What the project is drawn with next to its name. Absent: the app's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<ProjectIcon>,
}

/// The picture of a project: an icon of the app's icon set or an emoji.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProjectIcon {
    /// An icon of the app's set by its kebab-case name (`"rocket"`, `"gamepad-2"`).
    /// The daemon checks only the form of the name: the set belongs to the app, and an
    /// app that does not know a name draws its default icon.
    Lucide {
        name: String,
        /// Absent: the color of the text around it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<IconColor>,
    },
    /// An emoji, which brings its own colors.
    Emoji { emoji: String },
}

/// A color from the app's palette. The app picks the shade for its light or dark
/// theme, so a project looks right in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum IconColor {
    Brand,
    Red,
    Orange,
    Yellow,
    Green,
    Teal,
    Blue,
    Violet,
    Pink,
    Gray,
}

/// The longest icon name the daemon accepts.
pub const ICON_NAME_MAX: usize = 48;
/// The most Unicode scalar values an emoji may have: enough for skin tones and
/// joined sequences (a couple kissing is ten), not for a sentence.
pub const ICON_EMOJI_MAX: usize = 16;

impl ProjectIcon {
    /// The icon as the daemon keeps it, or why it cannot be one. An emoji is trimmed;
    /// a name must be lowercase kebab-case, as the icon set spells its names.
    pub fn checked(self) -> Result<Self, String> {
        match self {
            Self::Lucide { name, color } => {
                let well_formed = !name.is_empty()
                    && name.len() <= ICON_NAME_MAX
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                    && !name.starts_with('-')
                    && !name.ends_with('-');
                if !well_formed {
                    return Err(format!(
                        "icon name {name:?} is not a kebab-case name of at most {ICON_NAME_MAX} characters"
                    ));
                }
                Ok(Self::Lucide { name, color })
            }
            Self::Emoji { emoji } => {
                let emoji = emoji.trim();
                if emoji.is_empty() {
                    return Err("an emoji icon cannot be empty".into());
                }
                if emoji.chars().count() > ICON_EMOJI_MAX {
                    return Err(format!("an emoji icon has at most {ICON_EMOJI_MAX} characters"));
                }
                if emoji.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return Err("an emoji icon cannot hold spaces or control characters".into());
                }
                Ok(Self::Emoji { emoji: emoji.to_owned() })
            }
        }
    }
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
            icon: None,
        }
    }

    fn lucide(name: &str) -> ProjectIcon {
        ProjectIcon::Lucide { name: name.into(), color: None }
    }

    fn emoji(emoji: &str) -> ProjectIcon {
        ProjectIcon::Emoji { emoji: emoji.into() }
    }

    #[test]
    fn icons_are_checked() {
        assert_eq!(lucide("gamepad-2").checked(), Ok(lucide("gamepad-2")));
        for bad in ["", "Rocket", "rocket icon", "-x", "x-", "x_y", &"a".repeat(ICON_NAME_MAX + 1)]
        {
            assert!(lucide(bad).checked().is_err(), "{bad:?} is not a name");
        }
        assert_eq!(emoji(" 🚀 ").checked(), Ok(emoji("🚀")));
        // Joined sequences with skin tones are one emoji.
        assert!(emoji("👩🏻\u{200d}❤️\u{200d}💋\u{200d}👨🏼").checked().is_ok());
        assert!(emoji("🇺🇦").checked().is_ok());
        for bad in ["", "  ", "🚀 🚀", "a\tb", &"🚀".repeat(ICON_EMOJI_MAX + 1)] {
            assert!(emoji(bad).checked().is_err(), "{bad:?} is not an emoji");
        }
    }

    #[test]
    fn a_project_without_an_icon_reads_and_writes_as_before() {
        let old = r#"{"id":"p","name":"App","folders":["/src"],"created_at":1}"#;
        let read: Project = serde_json::from_str(old).unwrap();
        assert_eq!(read.icon, None);
        assert_eq!(serde_json::to_string(&read).unwrap(), old);

        let icon = ProjectIcon::Lucide { name: "rocket".into(), color: Some(IconColor::Teal) };
        let json = serde_json::to_value(Project { icon: Some(icon.clone()), ..read }).unwrap();
        assert_eq!(
            json["icon"],
            serde_json::json!({"kind": "lucide", "name": "rocket", "color": "teal"})
        );
        assert_eq!(
            serde_json::to_value(emoji("🚀")).unwrap(),
            serde_json::json!({"kind": "emoji", "emoji": "🚀"})
        );
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
