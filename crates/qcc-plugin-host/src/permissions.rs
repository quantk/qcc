use serde::{Deserialize, Serialize};

/// What a plugin may do. In a manifest this is what the plugin *requests*; what it
/// gets is the intersection with what the user granted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Permissions {
    /// Hand deltas to the core. Only providers need it.
    pub deltas: bool,
    /// A private key-value store.
    pub kv: bool,
    pub timers: bool,
    /// Create, drive and read threads: orchestration.
    pub threads: bool,
    /// Offer skills (instructions) to every agent session.
    pub skills: bool,
    /// Hear of every turn that ends, in any thread.
    pub turn_events: bool,
    /// Leave comments on lines of threads' changes and read them.
    pub comments: bool,
    /// Read files in threads' working directories and in the plugin's own folder.
    pub files: bool,
    /// Folders under the user's home, relative to it (`.config/opencode`), whose
    /// files the plugin may read through `files` (which it needs as well).
    pub home_files: Vec<String>,
    /// Hear of every thread's changes (`thread-changed` events): what each agent
    /// does, and the start of what it says, across all threads.
    pub thread_events: bool,
    /// Show the user notifications on their desktop.
    pub notifications: bool,
    /// Executables the plugin may run, matched exactly against `spawn`'s `program`.
    pub process: Vec<String>,
}

impl Permissions {
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            deltas: self.deltas && other.deltas,
            kv: self.kv && other.kv,
            timers: self.timers && other.timers,
            threads: self.threads && other.threads,
            skills: self.skills && other.skills,
            turn_events: self.turn_events && other.turn_events,
            comments: self.comments && other.comments,
            files: self.files && other.files,
            home_files: self
                .home_files
                .iter()
                .filter(|dir| other.home_files.contains(dir))
                .cloned()
                .collect(),
            thread_events: self.thread_events && other.thread_events,
            notifications: self.notifications && other.notifications,
            process: self.process.iter().filter(|p| other.process.contains(p)).cloned().collect(),
        }
    }

    /// Whether `path`, relative to the home folder, is inside one of the folders
    /// `home_files` names. The path is taken as written: one with `..` or an
    /// absolute one is never inside.
    pub fn allows_home_path(&self, path: &str) -> bool {
        use std::path::{Component, Path};
        let path = Path::new(path);
        if !path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            return false;
        }
        let path: std::path::PathBuf =
            path.components().filter(|c| matches!(c, Component::Normal(_))).collect();
        self.home_files.iter().any(|dir| {
            let dir = Path::new(dir);
            dir.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
                && dir.components().next().is_some()
                && path.starts_with(dir)
        })
    }

    pub fn allows_process(&self, program: &str) -> bool {
        self.process.iter().any(|allowed| allowed == program)
    }
}

/// The user's side of the permission handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Grant {
    /// Everything the manifest requests. For plugins shipped with the daemon.
    AllRequested,
    Only(Permissions),
}

impl Grant {
    pub fn effective(&self, requested: &Permissions) -> Permissions {
        match self {
            Self::AllRequested => requested.clone(),
            Self::Only(granted) => requested.intersect(granted),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_paths_stay_inside_the_named_folders() {
        let permissions = Permissions {
            home_files: vec![".config/opencode".into(), ".claude/skills".into()],
            ..Permissions::default()
        };
        for allowed in
            [".config/opencode", ".config/opencode/skills/x/SKILL.md", "./.claude/skills/a.md"]
        {
            assert!(permissions.allows_home_path(allowed), "{allowed}");
        }
        for refused in [
            ".config",
            ".config/opencode-other/x",
            ".config/opencode/../../.ssh/id_rsa",
            "/home/user/.config/opencode",
            ".ssh/id_rsa",
            "",
        ] {
            assert!(!permissions.allows_home_path(refused), "{refused}");
        }
        assert!(!Permissions::default().allows_home_path(".config/opencode"));
        // A grant narrows the folders like any other permission.
        let granted =
            Permissions { home_files: vec![".claude/skills".into()], ..Permissions::default() };
        assert_eq!(permissions.intersect(&granted).home_files, [".claude/skills"]);
    }
}
