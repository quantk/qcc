//! Which skills Claude Code finds by itself, asked of the CLI: `reload_skills`
//! answers with the skills of the session, the same list `/skills` shows. Nothing
//! is sent to the model; the process is killed once it answered.

use qcc_plugin_sdk::types::{AgentSkill, Outcome};
use serde_json::Value;

use crate::probe::{self, ControlProbe};

const REQUEST_ID: &str = "qcc-skills";

/// The lines that ask: the handshake, then the question.
pub fn requests() -> Vec<String> {
    vec![
        probe::request("qcc-initialize", "initialize"),
        probe::request(REQUEST_ID, "reload_skills"),
    ]
}

pub fn probe() -> ControlProbe {
    ControlProbe {
        request_id: REQUEST_ID,
        what: "skills",
        parse: |response| Outcome::Skills(parse(&response["skills"])),
    }
}

/// Claude Code says where a skill is from at the end of its description:
/// `… (user)`, `… (claude.ai sync)`.
fn parse(skills: &Value) -> Vec<AgentSkill> {
    let entries = skills.as_array().map(Vec::as_slice).unwrap_or_default();
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str()?.to_owned();
            let text = entry["description"].as_str().unwrap_or("").trim();
            let (description, source) = match text
                .strip_suffix(')')
                .and_then(|t| t.rsplit_once(" ("))
            {
                Some((description, source)) => (description.to_owned(), Some(source.to_owned())),
                None => (text.to_owned(), None),
            };
            Some(AgentSkill { name, description, source, path: None })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use qcc_plugin_sdk::stdio::ProbeParser;
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_skills_and_where_each_is_from() {
        let line = json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": REQUEST_ID,
        "response": {"skills": [
            {"name": "tdd", "description": "Test first. (user)", "argumentHint": ""},
            {"name": "docs", "description": "Write docs (claude.ai sync)"},
            {"name": "bare", "description": "No source"},
        ]}}});
        let Some(Ok(Outcome::Skills(skills))) = probe().on_line(&line.to_string()) else {
            panic!("no skills");
        };
        let summary: Vec<_> = skills
            .iter()
            .map(|s| (s.name.as_str(), s.description.as_str(), s.source.as_deref()))
            .collect();
        assert_eq!(
            summary,
            [
                ("tdd", "Test first.", Some("user")),
                ("docs", "Write docs", Some("claude.ai sync")),
                ("bare", "No source", None),
            ]
        );
    }

    #[test]
    fn waits_for_its_own_answer() {
        let other = json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": "qcc-initialize", "response": {}}});
        assert!(probe().on_line(&other.to_string()).is_none());
    }
}
