//! What plugins declare about the extension points they contribute.

use serde::{Deserialize, Serialize};

use crate::ids::ProviderId;

/// How much an agent may do without asking the user. A thread without a mode runs
/// with the agent's own configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, specta::Type)]
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
    pub fork: bool,
    /// Input sent while a turn is running can be fed into that turn.
    pub steer: bool,
    /// The permission modes the provider can enforce.
    #[serde(default)]
    pub permission_modes: Vec<PermissionMode>,
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
}

/// Where a plugin's view appears in the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum UiPlacement {
    /// A screen of its own, next to the overview.
    App,
    /// A tab of the thread view.
    ThreadTab,
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
    pub id: String,
    pub name: String,
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

/// Instructions for agents that a plugin offers to every agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillInfo {
    pub name: String,
    /// When the agent should read it.
    pub description: String,
    /// Markdown.
    pub body: String,
}

/// A skill as an agent session gets it: a file it can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SkillRef {
    pub name: String,
    pub description: String,
    /// Absolute path of the skill's `SKILL.md`.
    pub path: String,
}
