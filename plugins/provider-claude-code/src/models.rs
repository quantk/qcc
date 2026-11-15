//! Which models Claude Code offers, asked of the CLI itself.
//!
//! The answer to the stream-json `initialize` control request lists the models the
//! account can use, the same list `/model` shows. Asking costs nothing: no prompt is
//! sent, and the process is killed once it answered.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::{ModelInfo, Outcome};
use serde_json::{Value, json};

const REQUEST_ID: &str = "qcc-models";

/// The arguments the probe runs `claude` with.
pub const ARGS: &[&str] =
    &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"];

/// The line that asks.
pub fn request() -> String {
    json!({"type": "control_request", "request_id": REQUEST_ID, "request": {"subtype": "initialize"}})
        .to_string()
}

#[derive(Default)]
pub struct ModelsProbe;

impl ProbeParser for ModelsProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let message: Value = serde_json::from_str(line).ok()?;
        let response = &message["response"];
        if message["type"] != "control_response" || response["request_id"] != REQUEST_ID {
            return None;
        }
        if response["subtype"] != "success" {
            let error = response["error"].as_str().unwrap_or("the request failed");
            return Some(Err(format!("claude could not list its models: {error}")));
        }
        Some(Ok(Outcome::Models(parse(&response["response"]["models"]))))
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        Err(exit_reason("claude", code, stderr))
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
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(line: &str) -> Option<Result<Outcome, String>> {
        ModelsProbe.on_line(line)
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
                    {"value": "x", "resolvedModel": "claude-x-1", "displayName": "X 1", "description": "Big"},
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
