//! The skills and slash commands an ACP agent finds on disk, read the way the agent
//! reads them. ACP announces them only inside a session (`available_commands_update`),
//! so to list them without one the plugin reads the agent's folders itself, through
//! the `files` import: the project's in the thread's working directory, and the
//! user's in the home folders its `home-files` permission names.
//!
//! The reading is written against [`Fs`], so it is tested without a host.

use qcc_plugin_sdk::types::{AgentCommand, AgentSkill};
use serde_json::Value;

/// Where a folder is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// Under the user's home folder.
    Home,
    /// In the working directory asked about.
    Project,
}

impl Place {
    fn source(self) -> &'static str {
        match self {
            Self::Home => "user",
            Self::Project => "project",
        }
    }
}

/// A folder or file the agent reads, relative to its place.
pub struct Location {
    pub place: Place,
    pub path: &'static str,
}

/// What an agent keeps on disk, by kind. Each list goes from what wins to what
/// loses when two have the same name.
pub struct Catalog {
    /// Folders of skills: each a folder with a `SKILL.md`, or a `.md` file.
    pub skills: &'static [Location],
    /// Folders of commands: each a `.md` file, named by its path without `.md`
    /// (`team/review.md` is `/team/review`).
    pub commands: &'static [Location],
    /// JSON configuration files with a `commands` (or `command`) table.
    pub configs: &'static [Location],
    /// Commands built into the agent: name, description, argument hint.
    pub builtin: &'static [(&'static str, &'static str, Option<&'static str>)],
}

/// How deep folders of commands and skills are looked into.
const MAX_DEPTH: usize = 4;

/// The reads the catalog needs. Missing folders and files are empty, not errors.
pub trait Fs {
    /// Names in a folder, sorted; empty for a file or nothing.
    fn list(&self, place: Place, dir: &str) -> Vec<String>;
    fn read(&self, place: Place, path: &str) -> Option<String>;
}

impl Catalog {
    /// The skills, each name once, by name.
    pub fn skills(&self, fs: &dyn Fs, project: bool) -> Vec<AgentSkill> {
        let mut found: Vec<AgentSkill> = Vec::new();
        for location in self.locations(self.skills, project) {
            for (name, description, path) in skills_in(fs, location.place, location.path, 0) {
                if !found.iter().any(|known| known.name == name) {
                    found.push(AgentSkill {
                        name,
                        description,
                        source: Some(location.place.source().to_owned()),
                        path: Some(path),
                    });
                }
            }
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found
    }

    /// The commands, each name once, by name; the built-in ones last of their name.
    pub fn commands(&self, fs: &dyn Fs, project: bool) -> Vec<AgentCommand> {
        let mut found: Vec<AgentCommand> = Vec::new();
        let mut add = |command: AgentCommand| {
            if !found.iter().any(|known| known.name == command.name) {
                found.push(command);
            }
        };
        for location in self.locations(self.commands, project) {
            for (name, description, hint) in commands_in(fs, location.place, location.path, "", 0) {
                let source = Some(location.place.source().to_owned());
                add(AgentCommand { name, description, argument_hint: hint, source });
            }
        }
        for location in self.locations(self.configs, project) {
            let Some(text) = fs.read(location.place, location.path) else { continue };
            for (name, description) in config_commands(&text) {
                let source = Some(location.place.source().to_owned());
                add(AgentCommand { name, description, argument_hint: None, source });
            }
        }
        for (name, description, hint) in self.builtin {
            add(AgentCommand {
                name: (*name).to_owned(),
                description: (*description).to_owned(),
                argument_hint: hint.map(str::to_owned),
                source: Some("built-in".to_owned()),
            });
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        found
    }

    /// The locations to read: the project's only when there is a project.
    fn locations<'a>(
        &self,
        locations: &'a [Location],
        project: bool,
    ) -> impl Iterator<Item = &'a Location> {
        locations.iter().filter(move |location| project || location.place == Place::Home)
    }
}

/// The skills in a folder: (name, description, path of its file).
fn skills_in(fs: &dyn Fs, place: Place, dir: &str, depth: usize) -> Vec<(String, String, String)> {
    let mut found = Vec::new();
    for entry in fs.list(place, dir) {
        if entry.starts_with('.') || entry == "node_modules" {
            continue;
        }
        let path = format!("{dir}/{entry}");
        let skill_file = format!("{path}/SKILL.md");
        if let Some(text) = fs.read(place, &skill_file) {
            let meta = front_matter(&text);
            let name = meta.get("name").cloned().unwrap_or_else(|| entry.clone());
            found.push((name, meta.get("description").cloned().unwrap_or_default(), skill_file));
        } else if let Some(stem) = entry.strip_suffix(".md") {
            let Some(text) = fs.read(place, &path) else { continue };
            let meta = front_matter(&text);
            let name = meta.get("name").cloned().unwrap_or_else(|| stem.to_owned());
            found.push((name, meta.get("description").cloned().unwrap_or_default(), path));
        } else if depth + 1 < MAX_DEPTH {
            found.extend(skills_in(fs, place, &path, depth + 1));
        }
    }
    found
}

/// The commands in a folder: (name, description, argument hint).
fn commands_in(
    fs: &dyn Fs,
    place: Place,
    dir: &str,
    prefix: &str,
    depth: usize,
) -> Vec<(String, String, Option<String>)> {
    let mut found = Vec::new();
    for entry in fs.list(place, dir) {
        if entry.starts_with('.') {
            continue;
        }
        let path = format!("{dir}/{entry}");
        if let Some(stem) = entry.strip_suffix(".md") {
            let Some(text) = fs.read(place, &path) else { continue };
            let meta = front_matter(&text);
            let hint = meta.get("argument-hint").cloned();
            found.push((
                format!("{prefix}{stem}"),
                meta.get("description").cloned().unwrap_or_default(),
                hint,
            ));
        } else if depth + 1 < MAX_DEPTH {
            found.extend(commands_in(fs, place, &path, &format!("{prefix}{entry}/"), depth + 1));
        }
    }
    found
}

/// The commands of a JSON configuration: (name, description). Comments on lines of
/// their own (JSONC) are skipped; a file that still does not parse has none.
fn config_commands(text: &str) -> Vec<(String, String)> {
    let plain: String = text
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let Ok(config) = serde_json::from_str::<Value>(&plain) else { return Vec::new() };
    let table = config.get("commands").or_else(|| config.get("command"));
    let Some(table) = table.and_then(Value::as_object) else { return Vec::new() };
    table
        .iter()
        .map(|(name, entry)| {
            let description = entry["description"].as_str().unwrap_or("").to_owned();
            (name.clone(), description)
        })
        .collect()
}

/// The fields of a Markdown file's YAML front matter that are plain strings, the
/// way skills and commands use them. A folded or literal block (`>`, `|`) is read
/// as its lines joined by spaces.
pub fn front_matter(text: &str) -> std::collections::HashMap<String, String> {
    let mut fields = std::collections::HashMap::new();
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return fields;
    }
    let mut current: Option<(String, Vec<String>)> = None;
    let mut finish = |current: &mut Option<(String, Vec<String>)>| {
        if let Some((key, parts)) = current.take() {
            fields.insert(key, parts.join(" "));
        }
    };
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented {
            if let Some((_, parts)) = current.as_mut() {
                parts.push(line.trim().to_owned());
            }
            continue;
        }
        finish(&mut current);
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim();
        let value = match value {
            ">" | "|" | ">-" | "|-" | "" => String::new(),
            _ => unquote(value).to_owned(),
        };
        let parts = if value.is_empty() { Vec::new() } else { vec![value] };
        current = Some((key.trim().to_owned(), parts));
    }
    finish(&mut current);
    fields
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// Files by place and path.
    #[derive(Default)]
    struct Memory(BTreeMap<(bool, String), String>);

    impl Memory {
        fn with(mut self, place: Place, path: &str, text: &str) -> Self {
            self.0.insert((place == Place::Home, path.to_owned()), text.to_owned());
            self
        }
    }

    impl Fs for Memory {
        fn list(&self, place: Place, dir: &str) -> Vec<String> {
            let prefix = format!("{dir}/");
            let mut names: Vec<String> = self
                .0
                .keys()
                .filter(|(home, _)| *home == (place == Place::Home))
                .filter_map(|(_, path)| path.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap_or(rest).to_owned())
                .collect();
            names.dedup();
            names
        }

        fn read(&self, place: Place, path: &str) -> Option<String> {
            self.0.get(&(place == Place::Home, path.to_owned())).cloned()
        }
    }

    const CATALOG: Catalog = Catalog {
        skills: &[
            Location { place: Place::Project, path: ".opencode/skills" },
            Location { place: Place::Home, path: ".config/opencode/skills" },
            Location { place: Place::Project, path: ".claude/skills" },
        ],
        commands: &[
            Location { place: Place::Project, path: ".opencode/commands" },
            Location { place: Place::Home, path: ".config/opencode/commands" },
        ],
        configs: &[Location { place: Place::Project, path: "opencode.json" }],
        builtin: &[
            ("init", "Set up AGENTS.md", None),
            ("review", "Review changes", Some("[commit]")),
        ],
    };

    fn skill(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\nBody")
    }

    #[test]
    fn reads_skills_where_the_agent_does_the_first_of_a_name_winning() {
        let fs = Memory::default()
            .with(Place::Project, ".opencode/skills/tdd/SKILL.md", &skill("tdd", "Project tdd"))
            .with(Place::Home, ".config/opencode/skills/tdd/SKILL.md", &skill("tdd", "User tdd"))
            .with(
                Place::Home,
                ".config/opencode/skills/notes.md",
                "---\ndescription: >\n  Keep\n  notes\n---\n",
            )
            .with(Place::Project, ".claude/skills/group/deep/SKILL.md", &skill("deep", "Nested"))
            .with(Place::Project, ".claude/skills/.hidden/SKILL.md", &skill("hidden", "No"));
        let skills = CATALOG.skills(&fs, true);
        let summary: Vec<_> = skills
            .iter()
            .map(|s| (s.name.as_str(), s.description.as_str(), s.source.as_deref().unwrap()))
            .collect();
        assert_eq!(
            summary,
            [
                ("deep", "Nested", "project"),
                ("notes", "Keep notes", "user"),
                ("tdd", "Project tdd", "project")
            ]
        );
        assert_eq!(skills[2].path.as_deref(), Some(".opencode/skills/tdd/SKILL.md"));
        // Without a project, only the user's.
        let names: Vec<_> = CATALOG.skills(&fs, false).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["notes", "tdd"]);
    }

    #[test]
    fn reads_commands_from_files_configs_and_the_built_in_list() {
        let fs = Memory::default()
            .with(Place::Project, ".opencode/commands/hello.md", "---\ndescription: \"Say hello\"\nargument-hint: <name>\n---\nHi $ARGUMENTS")
            .with(Place::Home, ".config/opencode/commands/team/review.md", "---\ndescription: Team review\n---\n")
            .with(Place::Home, ".config/opencode/commands/hello.md", "---\ndescription: Shadowed\n---\n")
            .with(Place::Project, "opencode.json", "{\n// comment\n\"commands\": {\"deploy\": {\"description\": \"Ship it\", \"template\": \"x\"}}}");
        let commands = CATALOG.commands(&fs, true);
        let summary: Vec<_> = commands
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.description.as_str(),
                    c.argument_hint.as_deref(),
                    c.source.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("deploy", "Ship it", None, "project"),
                ("hello", "Say hello", Some("<name>"), "project"),
                ("init", "Set up AGENTS.md", None, "built-in"),
                ("review", "Review changes", Some("[commit]"), "built-in"),
                ("team/review", "Team review", None, "user"),
            ]
        );
    }

    #[test]
    fn front_matter_takes_plain_quoted_and_folded_values() {
        let meta =
            front_matter("---\nname: 'x'\ndescription: |\n  one\n  two\nother: y\n---\nbody: no");
        assert_eq!(meta.get("name").map(String::as_str), Some("x"));
        assert_eq!(meta.get("description").map(String::as_str), Some("one two"));
        assert_eq!(meta.get("other").map(String::as_str), Some("y"));
        assert!(!meta.contains_key("body"));
        assert!(front_matter("no front matter").is_empty());
    }
}
