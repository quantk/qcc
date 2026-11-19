//! Reviewer profiles: markdown files with a frontmatter that says who reviews and
//! how, and a body that says what to look for.
//!
//! ```markdown
//! ---
//! name: security
//! description: Security problems in what changed
//! provider: codex
//! model: gpt-5.1-codex
//! effort: high
//! paths: [src/**/*.rs, "*.toml"]
//! severity: medium
//! delivery: auto
//! ---
//! Look only for security problems: …
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::findings::Severity;

/// Where a profile comes from. Later sources replace earlier ones of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    /// Shipped with the plugin.
    BuiltIn,
    /// `profiles/*.md` in the plugin's own folder of qcc's data.
    Global,
    /// `.qcc/reviewers/*.md` where the thread works.
    Project,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BuiltIn => "built in",
            Self::Global => "global",
            Self::Project => "project",
        })
    }
}

/// How a review's findings reach the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Delivery {
    /// Only as line comments; the user sends them to the agent.
    #[default]
    Collect,
    /// Also as one message to the agent once every reviewer of the round is done.
    Auto,
}

impl Delivery {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "collect" | "comments" => Some(Self::Collect),
            "auto" | "send" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collect => "collect",
            Self::Auto => "auto",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    /// Lowercase letters, digits, `-` and `_`; from the file name when not given.
    pub name: String,
    pub description: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Globs of the files the reviewer looks at; every file when empty.
    pub paths: Vec<String>,
    /// Findings below it are dropped.
    pub severity: Severity,
    /// Overrides the thread's choice.
    pub delivery: Option<Delivery>,
    /// What to look for.
    pub body: String,
    pub source: Source,
}

/// Profiles shipped with the plugin, as `(file name, text)`.
pub const BUILT_IN: [(&str, &str); 2] = [
    ("general.md", include_str!("../profiles/general.md")),
    ("security.md", include_str!("../profiles/security.md")),
];

/// A profile's name from its file name: `security.md` → `security`.
pub fn name_of_file(file: &str) -> Option<&str> {
    file.strip_suffix(".md").filter(|name| valid_name(name))
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Parse one profile file. `file_name` names it when the frontmatter does not.
pub fn parse(file_name: &str, text: &str, source: Source) -> Result<Profile, String> {
    let (front, body) = split_frontmatter(text);
    let mut profile = Profile {
        name: name_of_file(file_name).unwrap_or_default().to_owned(),
        description: None,
        provider: None,
        model: None,
        effort: None,
        paths: Vec::new(),
        severity: Severity::Low,
        delivery: None,
        body: body.trim().to_owned(),
        source,
    };
    let mut lines = front.lines().peekable();
    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("{file_name}: expected `key: value`, found {line:?}"));
        };
        let (key, value) = (key.trim(), value.trim());
        // A list may follow on lines of its own: `- item`.
        let mut items = Vec::new();
        if value.is_empty() {
            while let Some(next) = lines.peek() {
                let Some(item) = next.trim_start().strip_prefix("- ") else { break };
                items.push(unquote(item.trim()).to_owned());
                lines.next();
            }
        }
        let text = || (!value.is_empty()).then(|| unquote(value).to_owned());
        match key {
            "name" => match text() {
                Some(name) if valid_name(&name) => profile.name = name,
                _ => {
                    return Err(format!(
                        "{file_name}: name {value:?} must be lowercase letters, digits, - and _"
                    ));
                }
            },
            "description" => profile.description = text(),
            "provider" => profile.provider = text(),
            "model" => profile.model = text(),
            "effort" => profile.effort = text(),
            "paths" => {
                profile.paths = if value.is_empty() { items } else { inline_list(value) };
            }
            "severity" | "threshold" => {
                profile.severity = Severity::parse(value)
                    .ok_or_else(|| format!("{file_name}: unknown severity {value:?}"))?;
            }
            "delivery" => {
                profile.delivery = Some(Delivery::parse(value).ok_or_else(|| {
                    format!("{file_name}: delivery is collect or auto, not {value:?}")
                })?);
            }
            // Unknown keys are left for newer versions of the plugin.
            _ => {}
        }
    }
    if profile.name.is_empty() {
        return Err(format!("{file_name}: name the profile, in its frontmatter or its file name"));
    }
    if profile.body.is_empty() {
        return Err(format!("{file_name}: say what the reviewer looks for, below the frontmatter"));
    }
    Ok(profile)
}

/// The frontmatter between `---` lines at the top, and the rest.
fn split_frontmatter(text: &str) -> (&str, &str) {
    let text = text.trim_start_matches('\u{feff}');
    let Some(rest) = text.strip_prefix("---") else { return ("", text) };
    let rest = rest.trim_start_matches([' ', '\t', '\r']);
    let Some(rest) = rest.strip_prefix('\n') else { return ("", text) };
    match rest.find("\n---") {
        Some(end) => {
            let after = &rest[end + 4..];
            let body = after.split_once('\n').map_or("", |(_, body)| body);
            (&rest[..end], body)
        }
        None => ("", text),
    }
}

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

/// `[a, "b"]` or `a, b`.
fn inline_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let inner = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(value);
    inner
        .split(',')
        .map(|item| unquote(item.trim()).to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Profiles from every source; one of the same name from a later source replaces
/// the earlier. Sorted by name.
pub fn merge(sources: impl IntoIterator<Item = Profile>) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = Vec::new();
    for profile in sources {
        match profiles.iter_mut().find(|known| known.name == profile.name) {
            Some(known) => *known = profile,
            None => profiles.push(profile),
        }
    }
    profiles.sort_by(|a, b| a.name.cmp(&b.name));
    profiles
}

/// Whether `path` matches the glob: `*` is any run of characters but `/`, `**` any
/// run of whole folders, `?` one character. A glob without `/` matches the file
/// name in any folder.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    if !glob.contains('/') {
        let name = path.rsplit('/').next().unwrap_or(path);
        return matches_from(glob.as_bytes(), name.as_bytes());
    }
    matches_from(glob.trim_start_matches("./").as_bytes(), path.as_bytes())
}

fn matches_from(glob: &[u8], path: &[u8]) -> bool {
    match glob {
        [] => path.is_empty(),
        [b'*', b'*', b'/', rest @ ..] => {
            // Zero or more whole folders.
            if matches_from(rest, path) {
                return true;
            }
            path.iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'/')
                .any(|(index, _)| matches_from(rest, &path[index + 1..]))
        }
        [b'*', b'*'] => true,
        [b'*', rest @ ..] => (0..=path.len())
            .take_while(|&i| i == 0 || path[i - 1] != b'/')
            .any(|i| matches_from(rest, &path[i..])),
        [b'?', rest @ ..] => {
            matches!(path, [first, tail @ ..] if *first != b'/' && matches_from(rest, tail))
        }
        [byte, rest @ ..] => {
            matches!(path, [first, tail @ ..] if first == byte && matches_from(rest, tail))
        }
    }
}

/// Whether the profile looks at `path`.
pub fn covers(profile: &Profile, path: &str) -> bool {
    profile.paths.is_empty() || profile.paths.iter().any(|glob| glob_matches(glob, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_is_read_from_its_frontmatter_and_body() {
        let text = "---\nname: sec\ndescription: \"Security\"\nprovider: codex\nmodel: gpt-5\n\
                    effort: high\npaths: [src/**/*.rs, \"*.toml\"]\nseverity: medium\n\
                    delivery: auto\nfuture: ignored\n---\nLook for holes.\n";
        let profile = parse("whatever.md", text, Source::Project).unwrap();
        assert_eq!(profile.name, "sec");
        assert_eq!(profile.description.as_deref(), Some("Security"));
        assert_eq!(profile.provider.as_deref(), Some("codex"));
        assert_eq!(profile.model.as_deref(), Some("gpt-5"));
        assert_eq!(profile.effort.as_deref(), Some("high"));
        assert_eq!(profile.paths, ["src/**/*.rs", "*.toml"]);
        assert_eq!(profile.severity, Severity::Medium);
        assert_eq!(profile.delivery, Some(Delivery::Auto));
        assert_eq!(profile.body, "Look for holes.");

        let listed = "---\npaths:\n  - a/*.rs\n  - 'b/**'\n---\nbody";
        let profile = parse("style.md", listed, Source::Global).unwrap();
        assert_eq!(profile.name, "style", "named by its file");
        assert_eq!(profile.paths, ["a/*.rs", "b/**"]);
        assert_eq!(profile.severity, Severity::Low);

        let bare = parse("plain.md", "Just a body", Source::Global).unwrap();
        assert_eq!((bare.name.as_str(), bare.body.as_str()), ("plain", "Just a body"));
    }

    #[test]
    fn bad_profiles_say_what_is_wrong() {
        assert!(
            parse("x.md", "---\nseverity: dire\n---\nb", Source::Global)
                .unwrap_err()
                .contains("severity")
        );
        assert!(
            parse("x.md", "---\ndelivery: maybe\n---\nb", Source::Global)
                .unwrap_err()
                .contains("delivery")
        );
        assert!(
            parse("x.md", "---\nname: Bad Name\n---\nb", Source::Global)
                .unwrap_err()
                .contains("name")
        );
        assert!(
            parse("x.md", "---\nname: ok\n---\n", Source::Global)
                .unwrap_err()
                .contains("looks for")
        );
        assert!(parse("X Y.md", "body", Source::Global).unwrap_err().contains("name the profile"));
    }

    #[test]
    fn built_in_profiles_parse() {
        for (file, text) in BUILT_IN {
            let profile = parse(file, text, Source::BuiltIn).unwrap();
            assert!(profile.description.is_some(), "{file}");
        }
    }

    #[test]
    fn later_sources_replace_earlier_ones() {
        let make = |name: &str, source| parse(&format!("{name}.md"), "body", source).unwrap();
        let merged = merge([
            make("general", Source::BuiltIn),
            make("b", Source::Global),
            make("general", Source::Project),
        ]);
        let names: Vec<_> = merged.iter().map(|p| (p.name.as_str(), p.source)).collect();
        assert_eq!(names, [("b", Source::Global), ("general", Source::Project)]);
    }

    #[test]
    fn globs_match_paths() {
        assert!(glob_matches("*.rs", "src/deep/lib.rs"));
        assert!(!glob_matches("*.rs", "src/lib.ts"));
        assert!(glob_matches("src/*.rs", "src/lib.rs"));
        assert!(!glob_matches("src/*.rs", "src/a/lib.rs"));
        assert!(glob_matches("src/**/*.rs", "src/lib.rs"));
        assert!(glob_matches("src/**/*.rs", "src/a/b/lib.rs"));
        assert!(glob_matches("src/**", "src/a/b/lib.rs"));
        assert!(glob_matches("a?c.md", "abc.md"));
        assert!(!glob_matches("docs/*.md", "src/docs/a.md"));
    }
}
