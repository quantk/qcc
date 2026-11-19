//! The findings of a thread's reviews, round after round, and where each stands.
//!
//! A finding on a line of the changes lives on as a comment there, and its status
//! is the comment's: sent with a message, resolved by the agent, or open. The user
//! dismissing one resolves the comment too, so that the agent sees it closed; the
//! finding remembers that it was dismissed, not fixed. A finding with no line (a
//! remark about the whole) has no comment and keeps its status here.

use serde::{Deserialize, Serialize};

use crate::findings::Severity;

/// What the user's dismissal says on the comment, for the agent to read.
pub const DISMISSED_NOTE: &str = "Dismissed by the user";

/// Where a finding stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Nobody acted on it yet.
    Open,
    /// Handed to the agent with a message.
    Sent,
    /// The agent (or the user) resolved its comment.
    Fixed,
    /// The user decided it needs nothing.
    Dismissed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Sent => "sent",
            Self::Fixed => "fixed",
            Self::Dismissed => "dismissed",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Sent => "Sent to the agent",
            Self::Fixed => "Fixed",
            Self::Dismissed => "Dismissed",
        }
    }

    /// Still something for the agent to do.
    pub fn pending(self) -> bool {
        matches!(self, Self::Open | Self::Sent)
    }
}

/// One finding of one reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// `f1`, `f2`, … within the thread.
    pub id: String,
    pub round: u32,
    /// The profile that found it.
    pub reviewer: String,
    /// The reviewer's thread.
    pub critic: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub severity: Severity,
    pub text: String,
    /// The changes moved on while the reviewer looked.
    pub stale: bool,
    /// The comment that holds it.
    pub comment: Option<String>,
    /// For a finding without a comment: where it stands.
    pub status: Status,
    /// The user dismissed it; its comment is resolved with [`DISMISSED_NOTE`].
    #[serde(default)]
    pub dismissed: bool,
}

impl Record {
    pub fn location(&self) -> Option<String> {
        match (&self.file, self.line) {
            (Some(file), Some(line)) => Some(format!("{file}:{line}")),
            (Some(file), None) => Some(file.clone()),
            _ => None,
        }
    }
}

/// A comment as the ledger needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentState {
    pub id: String,
    pub resolved: bool,
    pub resolution: Option<String>,
    pub sent: bool,
}

/// Where a finding stands, and the agent's note when it was fixed.
pub fn status(record: &Record, comments: &[CommentState]) -> (Status, Option<String>) {
    let Some(id) = &record.comment else { return (record.status, None) };
    let Some(comment) = comments.iter().find(|comment| &comment.id == id) else {
        // Its comment was deleted: nobody is to act on it any more.
        return (Status::Dismissed, None);
    };
    match (comment.resolved, record.dismissed) {
        (true, true) => (Status::Dismissed, None),
        (true, false) => (Status::Fixed, comment.resolution.clone()),
        (false, _) if comment.sent => (Status::Sent, None),
        (false, _) => (Status::Open, None),
    }
}

/// A finding with where it stands and the agent's note on a fixed one.
pub type Standing = (Record, Status, Option<String>);

/// How the panel groups findings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Grouping {
    #[default]
    Reviewer,
    Severity,
}

impl Grouping {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "reviewer" => Some(Self::Reviewer),
            "severity" => Some(Self::Severity),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reviewer => "reviewer",
            Self::Severity => "severity",
        }
    }
}

/// Findings in groups, each group's title first: by reviewer in the order they
/// first spoke, or by severity from the gravest. Within a group, what is still to
/// do comes first, the newest round first, then the rest.
pub fn grouped(findings: &[Standing], by: Grouping) -> Vec<(String, Vec<&Standing>)> {
    let mut groups: Vec<(String, Vec<&Standing>)> = Vec::new();
    let key = |record: &Record| match by {
        Grouping::Reviewer => record.reviewer.clone(),
        Grouping::Severity => record.severity.as_str().to_owned(),
    };
    for finding in findings {
        let title = key(&finding.0);
        match groups.iter_mut().find(|(group, _)| *group == title) {
            Some((_, members)) => members.push(finding),
            None => groups.push((title, vec![finding])),
        }
    }
    if by == Grouping::Severity {
        groups.sort_by_key(|(title, _)| {
            std::cmp::Reverse(Severity::parse(title).unwrap_or_default())
        });
    }
    for (_, members) in &mut groups {
        members.sort_by_key(|(record, status, _)| {
            (!status.pending(), std::cmp::Reverse(record.round), record.id.clone())
        });
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, reviewer: &str, severity: Severity, comment: Option<&str>) -> Record {
        Record {
            id: id.into(),
            round: 1,
            reviewer: reviewer.into(),
            critic: "thr_c".into(),
            file: comment.map(|_| "a.rs".into()),
            line: comment.map(|_| 3),
            severity,
            text: "boom".into(),
            stale: false,
            comment: comment.map(Into::into),
            status: Status::Open,
            dismissed: false,
        }
    }

    fn comment(id: &str, resolved: bool, sent: bool, note: Option<&str>) -> CommentState {
        CommentState { id: id.into(), resolved, resolution: note.map(Into::into), sent }
    }

    #[test]
    fn a_finding_on_a_line_stands_where_its_comment_does() {
        let found = record("f1", "general", Severity::High, Some("c1"));
        assert_eq!(status(&found, &[comment("c1", false, false, None)]).0, Status::Open);
        assert_eq!(status(&found, &[comment("c1", false, true, None)]).0, Status::Sent);
        let fixed = status(&found, &[comment("c1", true, true, Some("renamed it"))]);
        assert_eq!(fixed, (Status::Fixed, Some("renamed it".into())));
        let dismissed = Record { dismissed: true, ..found.clone() };
        let note = Some(DISMISSED_NOTE);
        assert_eq!(status(&dismissed, &[comment("c1", true, false, note)]).0, Status::Dismissed);
        // Reopened in Changes after a dismissal: open again.
        assert_eq!(status(&dismissed, &[comment("c1", false, false, None)]).0, Status::Open);
        assert_eq!(status(&found, &[]).0, Status::Dismissed, "its comment was deleted");
    }

    #[test]
    fn a_remark_keeps_its_own_status() {
        let remark =
            Record { status: Status::Sent, ..record("f2", "general", Severity::Low, None) };
        assert_eq!(status(&remark, &[]), (Status::Sent, None));
    }

    #[test]
    fn findings_group_by_reviewer_or_by_severity_with_the_open_ones_first() {
        let findings = vec![
            (record("f1", "general", Severity::Low, None), Status::Fixed, None),
            (record("f2", "security", Severity::High, None), Status::Open, None),
            (record("f3", "general", Severity::High, None), Status::Open, None),
        ];
        let by_reviewer = grouped(&findings, Grouping::Reviewer);
        let titles: Vec<&str> = by_reviewer.iter().map(|(title, _)| title.as_str()).collect();
        assert_eq!(titles, ["general", "security"]);
        let general: Vec<&str> = by_reviewer[0].1.iter().map(|f| f.0.id.as_str()).collect();
        assert_eq!(general, ["f3", "f1"], "what is still to do first");
        let by_severity = grouped(&findings, Grouping::Severity);
        let titles: Vec<&str> = by_severity.iter().map(|(title, _)| title.as_str()).collect();
        assert_eq!(titles, ["high", "low"]);
    }
}
