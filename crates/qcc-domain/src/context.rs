//! What an agent's context window is made of.

use serde::{Deserialize, Serialize};

/// What a part of the context window is, which says how it adds up: only `Used`
/// parts are in the context; the rest of the window is `Free` or held back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    /// Content the context holds.
    Used,
    /// What is left of the window.
    Free,
    /// Room the agent keeps for compacting; not available to the conversation.
    Buffer,
    /// Known to the agent but loaded only when needed, so not in the window now.
    Deferred,
}

/// One part of the context window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ContextCategory {
    /// What the part is. Agents share the well-known ids (`system_prompt`,
    /// `system_tools`, `mcp_tools`, `memory_files`, `skills`, `custom_agents`,
    /// `messages`, `autocompact_buffer`, `free_space`); a bridge may use its own.
    pub id: String,
    /// How the agent names the part.
    pub name: String,
    pub tokens: u64,
    pub kind: ContextKind,
}

/// One thing within a category: a memory file, an MCP tool, a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ContextEntry {
    /// The id of the category it belongs to.
    pub category: String,
    pub name: String,
    pub tokens: u64,
}

/// What the context window of a session holds, part by part, as its agent last
/// said or its bridge reckoned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ContextUsage {
    /// Tokens the context holds: the `Used` parts together.
    pub used: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u64>,
    /// The parts are reckoned rather than counted by the agent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub estimated: bool,
    /// The parts in the agent's order. Their tokens add up to the window when the
    /// agent reports free space, and to `used` when it does not.
    pub categories: Vec<ContextCategory>,
    /// What the larger parts are made of, largest first within a category. Kept
    /// in the timeline only; a thread's projection leaves them out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<ContextEntry>,
}

impl ContextUsage {
    /// The breakdown without its entries: what a thread's projection keeps, small
    /// enough to send with every thread.
    pub fn summary(&self) -> Self {
        Self { entries: Vec::new(), ..self.clone() }
    }
}
