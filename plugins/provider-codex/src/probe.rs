//! Questions asked of a short-lived `codex app-server`: the handshake, then one
//! request, whose response is the answer.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::Outcome;
use serde_json::{Value, json};

use crate::bridge::client_info;

/// The id of the list request.
pub const LIST_ID: u64 = 2;

/// The handshake and the question, one line each.
pub fn requests(method: &str) -> Vec<String> {
    requests_with(method, json!({}))
}

/// The handshake and the question with its parameters, one line each.
pub fn requests_with(method: &str, params: Value) -> Vec<String> {
    vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": client_info()}})
            .to_string(),
        json!({"jsonrpc": "2.0", "method": "initialized"}).to_string(),
        json!({"jsonrpc": "2.0", "id": LIST_ID, "method": method, "params": params}).to_string(),
    ]
}

/// Waits for the response to the list request and reads it with `parse`.
pub struct ListProbe {
    /// What is being listed, for the error message.
    pub what: &'static str,
    /// Reads the outcome out of the response's `result`.
    pub parse: fn(&Value) -> Outcome,
}

impl ProbeParser for ListProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let message: Value = serde_json::from_str(line).ok()?;
        if message["id"] != LIST_ID {
            return None;
        }
        if let Some(error) = message.get("error") {
            let error = error["message"].as_str().unwrap_or("the request failed");
            return Some(Err(format!("codex could not list its {}: {error}", self.what)));
        }
        Some(Ok((self.parse)(&message["result"])))
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        Err(exit_reason("codex", code, stderr))
    }
}
