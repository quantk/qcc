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
            process: self.process.iter().filter(|p| other.process.contains(p)).cloned().collect(),
        }
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
