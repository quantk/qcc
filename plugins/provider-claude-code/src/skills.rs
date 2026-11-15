//! Which skills Claude Code finds by itself, asked of the CLI: `reload_skills`
//! answers with the skills of the session, the same list `/skills` shows. Nothing
//! is sent to the model; the process is killed once it answered.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::{AgentSkill, Outcome};
use serde_json::{Value, json};

const REQUEST_ID: &str = "qcc-skills";

/// The lines that ask: the handshake, then the question.
pub fn requests() -> Vec<String> {
    let request = |id: &str, subtype: &str| {
        json!({"type": "control_request", "request_id": id, "request": {"subtype": subtype}})
            .to_string()
    };
    vec![request("qcc-initialize", "initialize"), request(REQUEST_ID, "reload_skills")]
}

#[derive(Default)]
pub struct SkillsProbe;

impl ProbeParser for SkillsProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let message: Value = serde_json::from_str(line).ok()?;
        let response = &message["response"];
        if message["type"] != "control_response" || response["request_id"] != REQUEST_ID {
            return None;
        }
        if response["subtype"] != "success" {
            let error = response["error"].as_str().unwrap_or("the request failed");
            return Some(Err(format!("claude could not list its skills: {error}")));
        }
        Some(Ok(Outcome::Skills(parse(&response["response"]["skills"]))))
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        Err(exit_reason("claude", code, stderr))
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
        let Some(Ok(Outcome::Skills(skills))) = SkillsProbe.on_line(&line.to_string()) else {
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
        assert!(SkillsProbe.on_line(&other.to_string()).is_none());
    }
}
