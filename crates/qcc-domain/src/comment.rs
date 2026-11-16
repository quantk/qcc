//! Comments the user leaves on lines of what a thread changed, for its agent to act on.

use serde::{Deserialize, Serialize};

use crate::ids::{CommentId, ThreadId};

/// Which side of a diff a line number counts on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum DiffSide {
    /// The code before the change: a deleted line.
    Old,
    /// The code after the change: an added or unchanged line.
    #[default]
    New,
}

/// A remark on one line of a thread's changes. It stays where it was left while
/// the code moves on, so it carries the line's text to be found by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LineComment {
    pub id: CommentId,
    pub thread_id: ThreadId,
    /// The file, as the thread's diff names it: from the top of the checkout.
    pub path: String,
    /// 1-based, counted on `side`.
    pub line: u32,
    pub side: DiffSide,
    /// The text of the line when the comment was left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    pub text: String,
    /// Milliseconds since the Unix epoch.
    pub created_at: u64,
    /// When it was dealt with; absent while it is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
    /// What was done about it, in the words of whoever resolved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

impl LineComment {
    pub fn is_open(&self) -> bool {
        self.resolved_at.is_none()
    }

    /// `path:line`, with a marker for a line that only the old code has.
    pub fn location(&self) -> String {
        match self.side {
            DiffSide::New => format!("{}:{}", self.path, self.line),
            DiffSide::Old => format!("{}:{} (removed line)", self.path, self.line),
        }
    }
}
