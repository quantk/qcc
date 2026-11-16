//! Which skills Codex finds by itself, asked of `codex app-server` with
//! `skills/list`. Skills the user turned off in Codex are left out.

use qcc_plugin_sdk::types::{AgentSkill, Outcome};
use serde_json::Value;

use crate::probe::{self, ListProbe};

/// The handshake and the question, one line each.
pub fn requests() -> Vec<String> {
    probe::requests("skills/list")
}

pub fn probe() -> ListProbe {
    ListProbe { what: "skills", parse: |data| Outcome::Skills(parse(data)) }
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
    use qcc_plugin_sdk::stdio::ProbeParser;
    use serde_json::json;

    use super::*;
    use crate::probe::LIST_ID;

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
        let Some(Ok(Outcome::Skills(skills))) = probe().on_line(&line.to_string()) else {
            panic!("no skills");
        };
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "tdd");
        assert_eq!(skills[0].source.as_deref(), Some("user"));
        assert_eq!(skills[0].path.as_deref(), Some("/s/tdd/SKILL.md"));
    }
}
