//! What a reviewer reports: a JSON object at the end of its last message.
//!
//! ```json
//! {"findings": [{"file": "src/lib.rs", "line": 12, "severity": "high", "text": "…"}]}
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How much a finding matters, from the least.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    #[default]
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub const ALL: [Self; 5] = [Self::Info, Self::Low, Self::Medium, Self::High, Self::Critical];

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim().trim_matches(['"', '\'']).to_ascii_lowercase().as_str() {
            "info" | "nit" | "note" => Self::Info,
            "low" | "minor" => Self::Low,
            "medium" | "moderate" => Self::Medium,
            "high" | "major" => Self::High,
            "critical" | "blocker" => Self::Critical,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    /// The file, as the diff names it; absent for a remark about the change as a whole.
    pub file: Option<String>,
    /// 1-based, in the file as it is now.
    pub line: Option<u32>,
    pub severity: Severity,
    pub text: String,
}

impl Finding {
    /// Where it is, for a message: `src/lib.rs:12`.
    pub fn location(&self) -> Option<String> {
        match (&self.file, self.line) {
            (Some(file), Some(line)) => Some(format!("{file}:{line}")),
            (Some(file), None) => Some(file.clone()),
            _ => None,
        }
    }
}

/// What a reviewer's reply says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Findings(Vec<Finding>),
    /// The reply has no findings object: the reviewer did not follow the format.
    Unstructured,
}

/// The findings of a reviewer's reply: the last `{"findings": [...]}` object in it,
/// fenced or not. Entries without text are dropped; a severity the reviewer made up
/// counts as low.
pub fn parse(reply: &str) -> Report {
    let starts: Vec<usize> = reply.match_indices('{').map(|(index, _)| index).collect();
    for start in starts.into_iter().rev() {
        let mut stream = serde_json::Deserializer::from_str(&reply[start..]).into_iter::<Value>();
        let Some(Ok(value)) = stream.next() else { continue };
        let Some(items) = value.get("findings").and_then(Value::as_array) else { continue };
        return Report::Findings(items.iter().filter_map(finding).collect());
    }
    Report::Unstructured
}

fn finding(item: &Value) -> Option<Finding> {
    let text = ["text", "message", "comment"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())?
        .to_owned();
    let file = ["file", "path"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
        .map(|file| file.trim().trim_start_matches("./").to_owned())
        .filter(|file| !file.is_empty());
    let line = item
        .get("line")
        .and_then(|line| line.as_u64().or_else(|| line.as_str()?.trim().parse().ok()))
        .and_then(|line| u32::try_from(line).ok())
        .filter(|line| *line > 0);
    let severity =
        item.get("severity").and_then(Value::as_str).and_then(Severity::parse).unwrap_or_default();
    Some(Finding { file, line, severity, text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_findings_object_counts() {
        let reply = "I looked. The format is {\"findings\": []}.\n\n```json\n{\"findings\": [\
                     {\"file\": \"./src/a.rs\", \"line\": 3, \"severity\": \"HIGH\", \"text\": \"off by one\"},\
                     {\"path\": \"b.rs\", \"line\": \"7\", \"message\": \"unclear\"},\
                     {\"file\": \"c.rs\", \"severity\": \"weird\", \"text\": \"whole file\"},\
                     {\"text\": \"  \"},\
                     {\"text\": \"overall: no tests\", \"severity\": \"medium\"}]}\n```\n";
        let Report::Findings(findings) = parse(reply) else { panic!("no findings") };
        assert_eq!(findings.len(), 4);
        assert_eq!(findings[0].location().as_deref(), Some("src/a.rs:3"));
        assert_eq!(findings[0].severity, Severity::High);
        assert_eq!((findings[1].line, findings[1].text.as_str()), (Some(7), "unclear"));
        assert_eq!(findings[1].severity, Severity::Low);
        assert_eq!(findings[2].location().as_deref(), Some("c.rs"));
        assert_eq!(findings[3].location(), None);
    }

    #[test]
    fn a_reply_without_the_object_is_unstructured() {
        assert_eq!(parse("Looks fine to me."), Report::Unstructured);
        assert_eq!(parse("{\"other\": 1}"), Report::Unstructured);
        assert_eq!(parse("words {\"findings\": []} words"), Report::Findings(Vec::new()));
    }

    #[test]
    fn severities_are_ordered() {
        assert!(Severity::Critical > Severity::High && Severity::Low > Severity::Info);
        assert_eq!(Severity::parse("nit"), Some(Severity::Info));
        assert_eq!(Severity::parse("blocker"), Some(Severity::Critical));
        assert_eq!(Severity::parse("?"), None);
    }
}
