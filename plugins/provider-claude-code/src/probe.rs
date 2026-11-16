//! Questions asked of a short-lived `claude` process: one control request, whose
//! control response is the answer. Nothing is sent to the model; the process is
//! killed once it answered.

use qcc_plugin_sdk::stdio::{ProbeParser, exit_reason};
use qcc_plugin_sdk::types::Outcome;
use serde_json::{Value, json};

/// The arguments a probe runs `claude` with.
pub const ARGS: &[&str] =
    &["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"];

/// A control request line.
pub fn request(request_id: &str, subtype: &str) -> String {
    json!({"type": "control_request", "request_id": request_id, "request": {"subtype": subtype}})
        .to_string()
}

/// Waits for the control response to `request_id` and reads it with `parse`.
pub struct ControlProbe {
    pub request_id: &'static str,
    /// What is being listed, for the error message.
    pub what: &'static str,
    /// Reads the outcome out of a successful response's `response`.
    pub parse: fn(&Value) -> Outcome,
}

impl ProbeParser for ControlProbe {
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>> {
        let message: Value = serde_json::from_str(line).ok()?;
        let response = &message["response"];
        if message["type"] != "control_response" || response["request_id"] != self.request_id {
            return None;
        }
        if response["subtype"] != "success" {
            let error = response["error"].as_str().unwrap_or("the request failed");
            return Some(Err(format!("claude could not list its {}: {error}", self.what)));
        }
        Some(Ok((self.parse)(&response["response"])))
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String> {
        Err(exit_reason("claude", code, stderr))
    }
}
