//! What agents used to do their work: commands, tools, MCP tools and skills, how
//! often, how they ended and how long they took.

use serde::{Deserialize, Serialize};

use crate::ids::ThreadId;
use crate::item::{Item, ItemShape, ItemStatus};

/// What kind of thing an agent used.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, specta::Type,
)]
#[serde(rename_all = "camelCase")]
pub enum ToolKind {
    Command,
    FileChange,
    FileRead,
    Search,
    /// A tool of the agent's own.
    Tool,
    /// A tool of an MCP server.
    Mcp,
    /// A call that put a skill to use.
    Skill,
}

/// What one item says was used, if it is a use of anything.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ToolUse {
    pub kind: ToolKind,
    /// The program a command ran, or the tool's or skill's name; empty for file
    /// changes, reads and searches, which are counted by kind alone.
    pub name: String,
    /// The MCP server, for a tool of one.
    pub server: Option<String>,
}

impl ToolUse {
    pub fn of(item: &Item) -> Option<Self> {
        let (kind, name, server) = match &item.shape {
            ItemShape::Command { command, .. } => (ToolKind::Command, program(command), None),
            ItemShape::FileChange { .. } => (ToolKind::FileChange, String::new(), None),
            ItemShape::FileRead { .. } => (ToolKind::FileRead, String::new(), None),
            ItemShape::Search { .. } => (ToolKind::Search, String::new(), None),
            ItemShape::Tool { skill: Some(skill), .. } => (ToolKind::Skill, skill.clone(), None),
            ItemShape::Tool { name, mcp_server: Some(server), .. } => {
                (ToolKind::Mcp, name.clone(), Some(server.clone()))
            }
            ItemShape::Tool { name, .. } => (ToolKind::Tool, name.clone(), None),
            ItemShape::AgentMessage { .. }
            | ItemShape::Reasoning { .. }
            | ItemShape::Plan { .. }
            | ItemShape::Extension { .. } => return None,
        };
        Some(Self { kind, name, server })
    }
}

/// The program a command line runs: its first word that is not a variable set for
/// it, without the directory. Good enough to tell `cargo` from `git`.
fn program(command: &str) -> String {
    let word = command
        .split_whitespace()
        .find(|word| !word.contains('=') || word.starts_with(['-', '/', '.']))
        .unwrap_or("");
    word.rsplit('/').next().unwrap_or(word).chars().take(60).collect()
}

/// How the uses of one thing by one thread went in one span of time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ToolBucket {
    pub thread_id: ThreadId,
    /// Start of the span, in milliseconds since the Unix epoch.
    pub hour: u64,
    pub kind: ToolKind,
    /// See [`ToolUse::name`].
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// Uses that ended within the span.
    pub calls: u32,
    /// Ended in failure, a command with a non-zero exit code included.
    pub failed: u32,
    /// The user refused the approval it waited for.
    pub declined: u32,
    /// Cut off with its turn or session.
    pub interrupted: u32,
    /// From start to end, over the uses whose start is known.
    pub duration_ms: u64,
    /// The longest of them.
    pub longest_ms: u64,
}

impl ToolBucket {
    pub fn new(thread_id: ThreadId, hour: u64, tool: ToolUse) -> Self {
        Self {
            thread_id,
            hour,
            kind: tool.kind,
            name: tool.name,
            server: tool.server,
            calls: 0,
            failed: 0,
            declined: 0,
            interrupted: 0,
            duration_ms: 0,
            longest_ms: 0,
        }
    }

    /// Count one use that ended as `item` says, taking `duration` milliseconds when
    /// known.
    pub fn count(&mut self, item: &Item, duration: Option<u64>) {
        self.calls += 1;
        match item.status {
            ItemStatus::Failed => self.failed += 1,
            ItemStatus::Declined => self.declined += 1,
            ItemStatus::Interrupted => self.interrupted += 1,
            ItemStatus::Completed => {
                if matches!(item.shape, ItemShape::Command { exit_code: Some(code), .. } if code != 0)
                {
                    self.failed += 1;
                }
            }
            ItemStatus::Pending => {}
        }
        if let Some(duration) = duration {
            self.duration_ms += duration;
            self.longest_ms = self.longest_ms.max(duration);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ItemId;

    fn item(status: ItemStatus, shape: ItemShape) -> Item {
        Item { id: ItemId::new("i"), status, shape }
    }

    fn tool(name: &str, mcp_server: Option<&str>, skill: Option<&str>) -> ItemShape {
        ItemShape::Tool {
            name: name.into(),
            input: serde_json::Value::Null,
            output: None,
            mcp_server: mcp_server.map(Into::into),
            skill: skill.map(Into::into),
        }
    }

    #[test]
    fn uses_are_told_apart_by_what_the_bridge_said() {
        let of = |shape| ToolUse::of(&item(ItemStatus::Completed, shape)).map(|u| (u.kind, u.name));
        assert_eq!(of(tool("Skill", None, Some("tdd"))), Some((ToolKind::Skill, "tdd".into())));
        assert_eq!(
            of(tool("mcp__docs__search", Some("docs"), None)),
            Some((ToolKind::Mcp, "mcp__docs__search".into()))
        );
        assert_eq!(of(tool("TodoWrite", None, None)), Some((ToolKind::Tool, "TodoWrite".into())));
        assert_eq!(of(ItemShape::AgentMessage { text: "hi".into() }), None);
        let command = |line: &str| ItemShape::Command {
            command: line.into(),
            cwd: None,
            output: None,
            exit_code: None,
        };
        assert_eq!(of(command("RUST_LOG=debug /usr/bin/cargo test")).unwrap().1, "cargo");
        assert_eq!(of(command("git status")).unwrap().1, "git");
    }

    #[test]
    fn a_bucket_counts_how_uses_ended_and_took() {
        let use_ = ToolUse { kind: ToolKind::Command, name: "cargo".into(), server: None };
        let mut bucket = ToolBucket::new(ThreadId::new("t"), 0, use_);
        let command = |exit_code| ItemShape::Command {
            command: "cargo test".into(),
            cwd: None,
            output: None,
            exit_code,
        };
        bucket.count(&item(ItemStatus::Completed, command(Some(0))), Some(100));
        bucket.count(&item(ItemStatus::Completed, command(Some(101))), Some(300));
        bucket.count(&item(ItemStatus::Declined, command(None)), None);
        assert_eq!((bucket.calls, bucket.failed, bucket.declined), (3, 1, 1));
        assert_eq!((bucket.duration_ms, bucket.longest_ms), (400, 300));
    }
}
