//! What plugins declare about the extension points they contribute.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ids::ProviderId;

/// How much an agent may do without asking the user. A thread without a mode runs
/// with the agent's own configuration.
///
/// The variants are declared from the strictest to the loosest, and the ordering
/// follows that: `Ask < AcceptEdits < Auto < Full`. A thread a thread started may
/// never run looser than its parent, and this order is what "looser" means.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionMode {
    /// Everything the agent wants approved is asked of the user.
    Ask,
    /// File edits inside the working directory go through; the rest is asked.
    AcceptEdits,
    /// The provider's own automatic reviewer decides; the user is not asked.
    Auto,
    /// Nothing is asked and nothing is sandboxed.
    Full,
}

impl PermissionMode {
    pub const ALL: [Self; 4] = [Self::Ask, Self::AcceptEdits, Self::Auto, Self::Full];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AcceptEdits => "accept-edits",
            Self::Auto => "auto",
            Self::Full => "full",
        }
    }

    /// The loosest of `supported` that allows no more than `self`: how a mode is
    /// carried over to an agent that cannot enforce it exactly.
    pub fn fit(self, supported: &[Self]) -> Option<Self> {
        supported.iter().copied().filter(|mode| *mode <= self).max()
    }
}

impl std::str::FromStr for PermissionMode {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL.into_iter().find(|mode| mode.as_str() == text).ok_or_else(|| {
            format!("unknown permission mode {text:?}; use ask, accept-edits, auto or full")
        })
    }
}

/// Session-behaviour facts reported by the provider's own code.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderCapabilities {
    /// A session can be resumed from its provider session handle.
    pub resume: bool,
    /// Input sent while a turn is running can be fed into that turn.
    pub steer: bool,
    /// The permission modes the provider can enforce.
    #[serde(default)]
    pub permission_modes: Vec<PermissionMode>,
    /// The MCP servers set up in qcc can be changed in a running session: added,
    /// removed, turned on and off. Without it, a change waits for the thread's next
    /// session.
    #[serde(default)]
    pub mcp_live: bool,
    /// The session's context can be compacted when the user asks.
    #[serde(default)]
    pub compact: bool,
    /// The provider can tell which skills its agent finds by itself.
    #[serde(default)]
    pub list_skills: bool,
    /// The provider reports what its agent's context window holds, part by part
    /// (`context.usage` deltas). Without it, only how full the window is is known.
    #[serde(default)]
    pub context_usage: bool,
    /// The agent looks at attached images (PNG, JPEG, GIF, WebP) as images. Without
    /// it, it is told where they are saved, like any other attached file.
    #[serde(default)]
    pub images: bool,
    /// The provider reports its account's usage limits (`rate.limits` deltas) and
    /// can be asked for them without a session.
    #[serde(default)]
    pub rate_limits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EnvironmentInfo {
    pub id: String,
    pub name: String,
    /// It can also tell what changed in a directory it did not make, so threads
    /// without an environment can show their changes through it.
    #[serde(default)]
    pub inspects_directories: bool,
    /// It can be told where to make its directory and what to call its branch.
    #[serde(default)]
    pub custom_placement: bool,
}

/// Where a plugin's view appears in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UiPlacement {
    /// A screen of its own, next to the overview.
    App,
    /// A tab of the thread view.
    ThreadTab,
    /// A tab of the panel beside the thread view.
    ThreadPanel,
}

/// Which threads a thread tab is offered for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UiScope {
    AllThreads,
    /// Only threads of a provider the same plugin contributes.
    OwnThreads,
}

/// A view a plugin draws from the fixed set of UI nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UiViewInfo {
    pub id: String,
    pub title: String,
    pub placement: UiPlacement,
    pub scope: UiScope,
    /// A page of the plugin's web bundle shown instead of UI nodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ModelInfo {
    /// What the agent is started with.
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The model the agent uses when none is chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_default: bool,
    /// The effort levels the model takes, from the least to the most thinking, as
    /// the provider names them. Empty: effort cannot be chosen for it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The level the model runs at when none is chosen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
}

/// A skill an agent finds by itself (its own folders, its plugins), as it reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AgentSkill {
    pub name: String,
    pub description: String,
    /// Where the agent has it from, in its own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Its `SKILL.md`, when the agent says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// How the work of an environment goes back to the directory it was made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DeliverMode {
    /// As changes not yet saved there (uncommitted, for git).
    Apply,
    /// Saved in the environment and merged (a commit and a git merge).
    Merge,
}

/// How a file differs from what an environment started from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    /// New and not yet known to version control.
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ChangedFile {
    /// Relative to the root of the environment.
    pub path: String,
    pub kind: ChangeKind,
    /// Line counts; absent for binary files and untracked ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
}

/// What a thread changed in its environment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct EnvironmentChanges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// What the changes are measured against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub files: Vec<ChangedFile>,
    /// The revisions saved since the base (commits, for git), newest first. Their ids
    /// name the ends of a [`ChangeScope::Revisions`] range.
    #[serde(default)]
    pub revisions: Vec<Revision>,
}

/// A saved state of an environment's work: a commit, for git.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct Revision {
    pub id: String,
    /// One line about it (a commit's subject).
    pub title: String,
    /// When it was made, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<u64>,
}

/// Which part of a thread's work its changes tell.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ChangeScope {
    /// Everything since the base, saved or not.
    #[default]
    All,
    /// Only what is not saved yet (uncommitted, for git).
    Unsaved,
    /// Between revisions listed in [`EnvironmentChanges::revisions`].
    Revisions {
        /// The changes after this revision; absent: after the base.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        /// Up to this revision, included; absent: up to the directory as it is now.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        to: Option<String>,
    },
}

impl ChangeScope {
    pub fn is_all(&self) -> bool {
        *self == Self::All
    }
}

/// A named value a plugin command takes. Every value is text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandParam {
    pub name: String,
    pub label: String,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// A command a plugin contributes to the CLI and the app's command palette.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The command acts on a thread, which the caller names.
    pub thread: bool,
    pub params: Vec<CommandParam>,
}

/// Instructions for agents that a plugin offers to agent sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillInfo {
    pub name: String,
    /// When the agent should read it.
    pub description: String,
    /// Markdown.
    pub body: String,
    /// Offered only to threads that ask for it, not to every session.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_request: bool,
}

/// What a plugin's setting holds, which also says how a client asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum SettingKind {
    Text,
    /// `true` or `false`.
    Flag,
    Choice {
        options: Vec<String>,
    },
    /// The id of an agent provider.
    Provider,
    /// A model of the provider the setting `provider` holds.
    Model {
        provider: String,
    },
    /// An effort level of the model the setting `model` holds, for the provider
    /// the setting `provider` holds.
    Effort {
        provider: String,
        model: String,
    },
    PermissionMode,
}

/// A setting of a plugin's own. Every value is text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SettingField {
    /// Letters, digits, `-` and `_`; unique within the plugin.
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub kind: SettingKind,
    /// What the plugin is told while the user has not set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

impl SettingField {
    /// Why `value` cannot be this setting's, or `None` if it can. `providers` are
    /// the ids of the agent providers there are.
    pub fn problem(&self, value: &str, providers: &[&str]) -> Option<String> {
        let id = &self.id;
        match &self.kind {
            SettingKind::Flag if value != "true" && value != "false" => {
                Some(format!("{id} is true or false, not {value:?}"))
            }
            SettingKind::Choice { options } if !options.iter().any(|option| option == value) => {
                Some(format!("{id} is one of {}, not {value:?}", options.join(", ")))
            }
            SettingKind::Provider if !providers.contains(&value) => {
                Some(format!("{id}: no plugin provides {value:?}"))
            }
            SettingKind::PermissionMode => value.parse::<PermissionMode>().err(),
            _ => None,
        }
    }
}

/// A skill as an agent session gets it: a file it can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
    /// Absolute path of the skill's `SKILL.md`.
    pub path: String,
}

/// An MCP server an agent session connects to, on top of those the agent's own
/// configuration names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct McpServer {
    /// Letters, digits, `_` and `-`: agents use it in tool names and config keys.
    pub name: String,
    pub transport: McpTransport,
}

/// How an agent reaches an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransport {
    /// A program the agent starts and talks to over its stdin and stdout.
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    /// A server the agent reaches over streamable HTTP.
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
}

/// How an MCP server of a session is doing, as its agent reports it. Covers the
/// servers of the agent's own configuration too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct McpServerStatus {
    pub name: String,
    pub state: McpState,
    /// Why it failed, in the agent's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "kebab-case")]
pub enum McpState {
    /// The agent is still connecting.
    Starting,
    Ready,
    Failed,
    /// The server wants the user to sign in, in the agent itself.
    NeedsAuth,
    /// Turned off for this session.
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::PermissionMode::{AcceptEdits, Ask, Auto, Full};

    #[test]
    fn modes_are_ordered_from_strictest_to_loosest() {
        assert!(Ask < AcceptEdits && AcceptEdits < Auto && Auto < Full);
    }

    #[test]
    fn a_mode_fits_an_agent_that_cannot_enforce_it_by_tightening() {
        let without_auto = [Ask, AcceptEdits, Full];
        assert_eq!(Auto.fit(&without_auto), Some(AcceptEdits));
        assert_eq!(Full.fit(&without_auto), Some(Full));
        assert_eq!(Ask.fit(&[AcceptEdits, Full]), None);
    }
}
