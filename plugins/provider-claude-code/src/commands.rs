//! Which slash commands Claude Code runs from a message, asked of the CLI: the
//! `initialize` control request answers with the session's commands (built-in
//! ones, the user's and the project's `.claude/commands`, skills and plugins').
//! Nothing is sent to the model; the process is killed once it answered.
//!
//! A message that starts with `/name` is run as that command in stream-json mode,
//! as long as the message is a plain string (see `Bridge::prompt_message`). A few
//! built-in commands are left out: the ones qcc does itself, where running them
//! behind its back would leave it showing something else (`/model`, `/effort`,
//! `/clear`, `/rename`), and the ones that do nothing useful outside Claude Code's
//! own terminal UI.

use qcc_plugin_sdk::types::{AgentCommand, Outcome};
use serde_json::Value;

use crate::probe::{self, ControlProbe};

const REQUEST_ID: &str = "qcc-commands";

/// Built-in commands not offered in qcc, and why.
const LEFT_OUT: &[&str] = &[
    // qcc sets the model, the effort and the session itself.
    "model",
    "effort",
    "fast",
    "clear",
    "rename",
    // Only for the terminal UI, or not available in a headless session.
    "color",
    "focus",
    "heapdump",
    "insights",
    // Need a confirmation or sign-in qcc cannot show.
    "design",
    "design-consent",
    "design-revoke",
    "ultrareview",
    "usage-credits",
    "extra-usage",
    // Gone or internal.
    "agents",
    "workflow-launch-exec",
];

/// The handshake, which is the question.
pub fn requests() -> Vec<String> {
    vec![probe::request(REQUEST_ID, "initialize")]
}

pub fn probe() -> ControlProbe {
    ControlProbe {
        request_id: REQUEST_ID,
        what: "commands",
        parse: |response| Outcome::Commands(parse(&response["commands"])),
    }
}

fn parse(commands: &Value) -> Vec<AgentCommand> {
    let entries = commands.as_array().map(Vec::as_slice).unwrap_or_default();
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str()?;
            let builtin = entry["builtin"] == true;
            if name.starts_with('_') || (builtin && LEFT_OUT.contains(&name)) {
                return None;
            }
            let text = entry["description"].as_str().unwrap_or("").trim();
            // Like skills, the user's and the project's say where they are from at
            // the end of the description: `… (project)`.
            let (description, source) =
                match text.strip_suffix(')').and_then(|t| t.rsplit_once(" (")) {
                    Some((description, source)) if !builtin => {
                        (description.to_owned(), Some(source.to_owned()))
                    }
                    _ => (text.to_owned(), builtin.then(|| "built-in".to_owned())),
                };
            let hint = entry["argumentHint"].as_str().filter(|hint| !hint.is_empty());
            Some(AgentCommand {
                name: name.to_owned(),
                description,
                argument_hint: hint.map(str::to_owned),
                source,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use qcc_plugin_sdk::stdio::ProbeParser;
    use serde_json::json;

    use super::*;

    #[test]
    fn reads_the_commands_without_the_ones_qcc_does_itself() {
        let line = json!({"type": "control_response", "response": {
        "subtype": "success", "request_id": REQUEST_ID,
        "response": {"commands": [
            {"name": "hello", "description": "Say hello (project)", "argumentHint": "name"},
            {"name": "context", "description": "Show current context usage", "argumentHint": "", "builtin": true},
            {"name": "model", "description": "Set the AI model", "argumentHint": "<model>", "builtin": true},
            {"name": "__remote-workflow", "description": "Internal", "builtin": true},
            {"name": "compact", "description": "Free up context (by summarizing)", "builtin": true},
        ]}}});
        let Some(Ok(Outcome::Commands(commands))) = probe().on_line(&line.to_string()) else {
            panic!("no commands");
        };
        let summary: Vec<_> = commands
            .iter()
            .map(|c| {
                (
                    c.name.as_str(),
                    c.description.as_str(),
                    c.argument_hint.as_deref(),
                    c.source.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("hello", "Say hello", Some("name"), Some("project")),
                ("context", "Show current context usage", None, Some("built-in")),
                ("compact", "Free up context (by summarizing)", None, Some("built-in")),
            ]
        );
    }
}
