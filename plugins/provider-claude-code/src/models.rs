//! Which models Claude Code offers, asked of the CLI itself.
//!
//! The answer to the stream-json `initialize` control request lists the models the
//! account can use, the same list `/model` shows. Asking costs nothing: no prompt is
//! sent, and the process is killed once it answered.

use qcc_plugin_sdk::types::{ModelInfo, Outcome};
use serde_json::Value;

use crate::probe::{self, ControlProbe};

const REQUEST_ID: &str = "qcc-models";

/// The line that asks.
pub fn request() -> String {
    probe::request(REQUEST_ID, "initialize")
}

pub fn probe() -> ControlProbe {
    ControlProbe {
        request_id: REQUEST_ID,
        what: "models",
        parse: |response| Outcome::Models(parse(&response["models"])),
    }
}

/// The `default` entry is not a model but the absence of a choice; it only says which
/// model that is.
fn parse(models: &Value) -> Vec<ModelInfo> {
    let entries = models.as_array().map(Vec::as_slice).unwrap_or_default();
    let default = entries
        .iter()
        .find(|entry| entry["value"] == "default")
        .and_then(|entry| entry["resolvedModel"].as_str());
    let mut marked = false;
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry["value"].as_str().filter(|id| *id != "default")?;
            let resolved = entry["resolvedModel"].as_str().unwrap_or(id);
            // Several aliases may resolve to the default; the first one stands for it.
            let is_default = !marked && default == Some(resolved);
            marked |= is_default;
            Some(ModelInfo {
                id: id.to_owned(),
                name: entry["displayName"].as_str().unwrap_or(id).to_owned(),
                description: entry["description"].as_str().map(str::to_owned),
                is_default,
                efforts: strings(&entry["supportedEffortLevels"]),
                // Claude Code does not say which level a model runs at unasked.
                default_effort: None,
            })
        })
        .collect()
}

fn strings(list: &Value) -> Vec<String> {
    let items = list.as_array().map(Vec::as_slice).unwrap_or_default();
    items.iter().filter_map(|item| item.as_str().map(str::to_owned)).collect()
}

#[cfg(test)]
mod tests {
    use qcc_plugin_sdk::stdio::ProbeParser;
    use serde_json::json;

    use super::*;

    fn answer(line: &str) -> Option<Result<Outcome, String>> {
        probe().on_line(line)
    }

    #[test]
    fn reads_the_models_out_of_the_initialize_answer() {
        let line = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": REQUEST_ID,
                "response": {"models": [
                    {"value": "default", "resolvedModel": "claude-x-1", "displayName": "Default"},
                    {"value": "x", "resolvedModel": "claude-x-1", "displayName": "X 1", "description": "Big",
                     "supportedEffortLevels": ["low", "high"]},
                    {"value": "claude-x-1", "resolvedModel": "claude-x-1", "displayName": "X 1 (pinned)"},
                    {"value": "y", "resolvedModel": "claude-y-1"},
                ]},
            },
        });
        let Some(Ok(Outcome::Models(models))) = answer(&line.to_string()) else {
            panic!("no models");
        };
        let summary: Vec<_> =
            models.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.is_default)).collect();
        assert_eq!(
            summary,
            [("x", "X 1", true), ("claude-x-1", "X 1 (pinned)", false), ("y", "y", false)]
        );
        assert_eq!(models[0].description.as_deref(), Some("Big"));
        assert_eq!(models[0].efforts, ["low", "high"]);
        assert!(models[1].efforts.is_empty());
    }

    #[test]
    fn other_lines_are_not_the_answer() {
        assert!(answer(r#"{"type":"system","subtype":"init"}"#).is_none());
        assert!(answer("not json").is_none());
        let other =
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"1"}}"#;
        assert!(answer(other).is_none());
    }

    #[test]
    fn a_refusal_is_an_error() {
        let line = json!({"type": "control_response", "response": {
            "subtype": "error", "request_id": REQUEST_ID, "error": "nope"}});
        assert_eq!(
            answer(&line.to_string()),
            Some(Err("claude could not list its models: nope".into()))
        );
    }
}
