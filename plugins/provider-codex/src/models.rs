//! Which models Codex offers, asked of `codex app-server` with `model/list`.

use qcc_plugin_sdk::types::{ModelInfo, Outcome};
use serde_json::Value;

use crate::probe::{self, ListProbe};

/// The handshake and the question, one line each.
pub fn requests() -> Vec<String> {
    probe::requests("model/list")
}

pub fn probe() -> ListProbe {
    ListProbe { what: "models", parse: |result| Outcome::Models(parse(&result["data"])) }
}

fn parse(models: &Value) -> Vec<ModelInfo> {
    let entries = models.as_array().map(Vec::as_slice).unwrap_or_default();
    entries
        .iter()
        .filter(|entry| entry["hidden"] != true)
        .filter_map(|entry| {
            // `model` is what `thread/start` takes; `id` names the same thing.
            let id = entry["model"].as_str().or_else(|| entry["id"].as_str())?;
            Some(ModelInfo {
                id: id.to_owned(),
                name: entry["displayName"].as_str().unwrap_or(id).to_owned(),
                description: entry["description"].as_str().map(str::to_owned),
                is_default: entry["isDefault"] == true,
                efforts: efforts(&entry["supportedReasoningEfforts"]),
                default_effort: entry["defaultReasoningEffort"].as_str().map(str::to_owned),
            })
        })
        .collect()
}

/// The levels, each listed as `{"reasoningEffort": "low", "description": …}`.
fn efforts(list: &Value) -> Vec<String> {
    let items = list.as_array().map(Vec::as_slice).unwrap_or_default();
    items.iter().filter_map(|item| item["reasoningEffort"].as_str().map(str::to_owned)).collect()
}

#[cfg(test)]
mod tests {
    use qcc_plugin_sdk::stdio::ProbeParser;
    use serde_json::json;

    use super::*;
    use crate::probe::LIST_ID;

    #[test]
    fn reads_the_listed_models() {
        let line = json!({"id": LIST_ID, "result": {"data": [
            {"id": "a", "model": "a", "displayName": "A", "description": "First", "isDefault": true,
             "supportedReasoningEfforts": [{"reasoningEffort": "low"}, {"reasoningEffort": "high"}],
             "defaultReasoningEffort": "low"},
            {"id": "b", "model": "b", "hidden": true},
            {"id": "c"},
        ]}});
        let Some(Ok(Outcome::Models(models))) = probe().on_line(&line.to_string()) else {
            panic!("no models");
        };
        let summary: Vec<_> =
            models.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.is_default)).collect();
        assert_eq!(summary, [("a", "A", true), ("c", "c", false)]);
        assert_eq!(models[0].efforts, ["low", "high"]);
        assert_eq!(models[0].default_effort.as_deref(), Some("low"));
        assert!(models[1].efforts.is_empty());
    }

    #[test]
    fn waits_for_its_own_answer() {
        assert!(probe().on_line(r#"{"id":1,"result":{}}"#).is_none());
        assert!(probe().on_line(r#"{"method":"remoteControl/status/changed"}"#).is_none());
        let error = json!({"id": LIST_ID, "error": {"code": -1, "message": "no"}});
        assert_eq!(
            probe().on_line(&error.to_string()),
            Some(Err("codex could not list its models: no".into()))
        );
    }
}
