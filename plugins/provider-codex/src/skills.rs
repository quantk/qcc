//! Which skills Codex finds by itself, asked of `codex app-server` with
//! `skills/list`. Skills the user turned off in Codex are left out.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::{AgentSkill, Outcome};
use serde_json::{Value, json};

const LIST_ID: u64 = 2;

/// The handshake and the question, one line each.
pub fn requests() -> Vec<String> {
    let client = json!({"name": "qcc", "title": "qcc", "version": env!("CARGO_PKG_VERSION")});
    vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": client}})
            .to_string(),
        json!({"jsonrpc": "2.0", "method": "initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": LIST_ID, "method": "skills/list", "params": {}}).to_string(),
    ]
}

#[derive(Default)]
pub struct SkillsProbe;

impl ProbeParser for SkillsProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let message: Value = serde_json::from_str(line).ok()?;
        if message["id"] != LIST_ID {
            return None;
        }
        if let Some(error) = message.get("error") {
            let error = error["message"].as_str().unwrap_or("the request failed");
            return Some(Err(format!("codex could not list its skills: {error}")));
        }
        Some(Ok(Outcome::Skills(parse(&message["result"]["data"]))))
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        Err(exit_reason("codex", code, stderr))
    }
}

/// One entry per working directory asked about, each with its skills.
fn parse(entries: &Value) -> Vec<AgentSkill> {
    let mut skills: Vec<AgentSkill> = Vec::new();
    let entries = entries.as_array().map(Vec::as_slice).unwrap_or_default();
    for skill in entries.iter().flat_map(|entry| entry["skills"].as_array().into_iter().flatten()) {
        let Some(name) = skill["name"].as_str() else { continue };
        if skill["enabled"] == false {
            continue;
        }
        let path = skill["path"].as_str().map(str::to_owned);
        if skills.iter().any(|known| known.name == name && known.path == path) {
            continue;
        }
        skills.push(AgentSkill {
            name: name.to_owned(),
            description: skill["description"].as_str().unwrap_or("").to_owned(),
            source: skill["scope"].as_str().map(str::to_owned),
            path,
        });
    }
    skills
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_enabled_skills_once() {
        let skill = |name: &str, enabled: bool| {
            json!({"name": name, "description": "D", "path": format!("/s/{name}/SKILL.md"),
                   "scope": "user", "enabled": enabled})
        };
        let line = json!({"id": LIST_ID, "result": {"data": [
            {"cwd": "/a", "skills": [skill("tdd", true), skill("off", false)]},
            {"cwd": "/b", "skills": [skill("tdd", true)]},
        ]}});
        let Some(Ok(Outcome::Skills(skills))) = SkillsProbe.on_line(&line.to_string()) else {
            panic!("no skills");
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "tdd");
        assert_eq!(skills[0].source.as_deref(), Some("user"));
        assert_eq!(skills[0].path.as_deref(), Some("/s/tdd/SKILL.md"));
    }
}
