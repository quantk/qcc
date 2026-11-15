//! The Claude Code dialect: stream-json lines in, deltas and stream-json lines out.
//!
//! Verified against Claude Code 2.1.325; the recordings in `fixtures/` are what that
//! version actually sent. What the bridge relies on:
//!
//! * a user message written while no turn runs starts one, which ends with exactly
//!   one `result`; a message written during a turn is taken into that turn at the
//!   next step and gets no `result` of its own;
//! * every turn starts with a `system` message of subtype `init`;
//! * text streams as `stream_event` deltas, and each finished content block arrives
//!   again, complete, in an `assistant` message;
//! * a tool call is a `tool_use` block, its outcome a `tool_result` block in a `user`
//!   message;
//! * with `--permission-prompt-tool stdio` a permission prompt is a `control_request`
//!   of subtype `can_use_tool`, answered by a `control_response`.

use std::collections::{HashMap, HashSet};

use qcc_plugin_sdk::types::*;
use serde_json::{Value, json};

/// What the bridge wants done after one input.
#[derive(Debug, Default, PartialEq)]
pub struct Output {
    /// Lines for the agent's stdin, without the newline.
    pub writes: Vec<String>,
    pub deltas: Vec<Delta>,
}

struct Approval {
    tool_use_id: String,
    input: Value,
}

pub struct Bridge {
    /// The session id already reported to the core.
    session: Option<String>,
    turn_open: bool,
    /// Turns we settled ourselves on interrupt whose `result` is still to come;
    /// everything the agent says until then belongs to a turn that is over.
    draining: u32,
    /// Open tool items by tool-use id, kept to close them with the same shape.
    tools: HashMap<String, (String, Value)>,
    approvals: HashMap<String, Approval>,
    /// Tool calls the user refused: they settle as declined, not failed.
    denied: HashSet<String>,
    total: TokenUsage,
    interrupts: u32,
}

fn stream(channel: &str, parent: Option<&str>) -> ItemKey {
    // A sub-agent streams under its own key so it never continues the main message.
    match parent {
        Some(parent) => ItemKey::Anonymous(format!("{channel}:{parent}")),
        None => ItemKey::Anonymous(channel.to_owned()),
    }
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn str_field<'a>(value: &'a Value, name: &str) -> &'a str {
    value.get(name).and_then(Value::as_str).unwrap_or("")
}

/// The item a tool call is shown as. `output` is absent while the call is running.
fn tool_shape(name: &str, input: &Value, output: Option<String>) -> ItemShape {
    match name {
        "Bash" => ItemShape::Command(CommandItem {
            command: str_field(input, "command").to_owned(),
            cwd: None,
            output,
            exit_code: None,
        }),
        "Read" => ItemShape::FileRead(str_field(input, "file_path").to_owned()),
        "Write" | "Edit" | "MultiEdit" => ItemShape::FileChange(FileChangeItem {
            path: str_field(input, "file_path").to_owned(),
            diff: proposed_diff(name, input),
        }),
        "NotebookEdit" => ItemShape::FileChange(FileChangeItem {
            path: str_field(input, "notebook_path").to_owned(),
            diff: None,
        }),
        "Grep" | "Glob" => ItemShape::Search(str_field(input, "pattern").to_owned()),
        "WebSearch" => ItemShape::Search(str_field(input, "query").to_owned()),
        _ => ItemShape::Tool(ToolItem { name: name.to_owned(), input: input.to_string(), output }),
    }
}

fn prefixed(prefix: char, text: &str) -> impl Iterator<Item = String> + '_ {
    text.lines().map(move |line| format!("{prefix}{line}"))
}

/// What a file tool is about to do, from its input alone: the lines it removes and
/// the lines it adds. This is all there is to show while the call waits for approval.
fn proposed_diff(name: &str, input: &Value) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut edit = |edit: &Value| {
        lines.extend(prefixed('-', str_field(edit, "old_string")));
        lines.extend(prefixed('+', str_field(edit, "new_string")));
    };
    match name {
        "Write" => lines.extend(prefixed('+', str_field(input, "content"))),
        "Edit" => edit(input),
        "MultiEdit" => input["edits"].as_array().into_iter().flatten().for_each(edit),
        _ => {}
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// The patch a file tool actually applied, as Claude Code reports it next to the tool
/// result: real hunks with line numbers and context.
fn applied_diff(tool_use_result: &Value) -> Option<String> {
    let hunks = tool_use_result.get("structuredPatch")?.as_array()?;
    let mut lines = Vec::new();
    for hunk in hunks {
        let number = |name: &str| hunk.get(name).and_then(Value::as_u64).unwrap_or(0);
        lines.push(format!(
            "@@ -{},{} +{},{} @@",
            number("oldStart"),
            number("oldLines"),
            number("newStart"),
            number("newLines")
        ));
        lines.extend(hunk["lines"].as_array()?.iter().filter_map(Value::as_str).map(str::to_owned));
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn option(id: &str, label: &str, kind: OptionKind) -> InteractionOption {
    InteractionOption { id: id.into(), label: label.into(), kind }
}

impl Bridge {
    /// `session` is the id being resumed, if any: it needs no announcement.
    pub fn new(session: Option<String>) -> Self {
        Self {
            session,
            turn_open: false,
            draining: 0,
            tools: HashMap::new(),
            approvals: HashMap::new(),
            denied: HashSet::new(),
            total: TokenUsage { input_tokens: 0, output_tokens: 0, cached_input_tokens: None },
            interrupts: 0,
        }
    }

    /// The user sent a message that starts a turn.
    pub fn prompt(&mut self, text: &str) -> Output {
        let mut output = self.steer(text);
        if !self.turn_open {
            self.turn_open = true;
            output.deltas.push(Delta::TurnOpen);
        }
        output
    }

    /// The user sent a message for the running turn. On the wire it is the same
    /// message as a prompt: the agent takes it into the turn at its next step.
    pub fn steer(&mut self, text: &str) -> Output {
        let message = json!({"type": "user", "message": {"role": "user", "content": text}});
        Output { writes: vec![message.to_string()], deltas: Vec::new() }
    }

    /// Interrupt the running turn. The turn is settled right here: whatever the agent
    /// still says about it, up to and including its `result`, is ignored.
    pub fn interrupt(&mut self) -> Output {
        let mut output = Output::default();
        if !self.turn_open {
            return output;
        }
        self.interrupts += 1;
        let request = json!({
            "type": "control_request",
            "request_id": format!("qcc-interrupt-{}", self.interrupts),
            "request": {"subtype": "interrupt"},
        });
        output.writes.push(request.to_string());
        self.draining += 1;
        self.settle(TurnStatus::Interrupted, None, &mut output.deltas);
        output
    }

    /// The user answered a permission prompt.
    pub fn resolve(
        &mut self,
        request_id: &str,
        resolution: &InteractionResolution,
    ) -> Result<Output, String> {
        let approval = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| format!("no permission request {request_id:?} is pending"))?;
        let response = match resolution {
            InteractionResolution::Selected(option) if option == "allow" => {
                json!({"behavior": "allow", "updatedInput": approval.input})
            }
            other => {
                self.denied.insert(approval.tool_use_id);
                let message = match other {
                    InteractionResolution::Text(text) => text.clone(),
                    _ => "The user declined this action.".to_owned(),
                };
                json!({"behavior": "deny", "message": message})
            }
        };
        let line = json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response},
        });
        Ok(Output { writes: vec![line.to_string()], deltas: Vec::new() })
    }

    /// The session was stopped on request.
    pub fn stopped(&mut self) -> Vec<Delta> {
        let mut deltas = Vec::new();
        if self.turn_open {
            self.turn_open = false;
            deltas.push(boundary(TurnStatus::Interrupted, None));
        }
        deltas.push(Delta::SessionEnded(Some("stopped".into())));
        deltas
    }

    /// The process went away by itself.
    pub fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta> {
        let mut reason = match code {
            Some(code) => format!("claude exited with code {code}"),
            None => "claude was killed".to_owned(),
        };
        if let Some(stderr) = stderr {
            reason = format!("{reason}: {stderr}");
        }
        let mut deltas = Vec::new();
        if self.turn_open {
            self.turn_open = false;
            deltas.push(boundary(TurnStatus::Failed, Some(reason.clone())));
        } else if code != Some(0) {
            deltas.push(Delta::ProviderError(reason.clone()));
        }
        deltas.push(Delta::SessionEnded(Some(reason)));
        deltas
    }

    /// One line of the agent's stdout.
    pub fn on_line(&mut self, line: &str) -> Output {
        let mut output = Output::default();
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return output;
        };
        let kind = str_field(&message, "type");

        if kind == "system" {
            if str_field(&message, "subtype") == "init" {
                let id = str_field(&message, "session_id");
                if !id.is_empty() && self.session.as_deref() != Some(id) {
                    self.session = Some(id.to_owned());
                    output.deltas.push(Delta::SessionIdentity(id.to_owned()));
                }
                // A turn we did not start: a steered message arrived after its turn
                // had ended, so the agent runs it as a turn of its own.
                if !self.turn_open && self.draining == 0 {
                    self.turn_open = true;
                    output.deltas.push(Delta::TurnOpen);
                }
            }
            return output;
        }
        if self.draining > 0 {
            if kind == "result" {
                self.draining -= 1;
            }
            return output;
        }

        let parent = message.get("parent_tool_use_id").and_then(Value::as_str);
        match kind {
            "stream_event" => self.stream_event(&message["event"], parent, &mut output.deltas),
            "assistant" => {
                for block in message["message"]["content"].as_array().into_iter().flatten() {
                    self.assistant_block(block, parent, &mut output.deltas);
                }
            }
            "user" => {
                for block in message["message"]["content"].as_array().into_iter().flatten() {
                    if str_field(block, "type") == "tool_result" {
                        self.tool_result(block, &message["tool_use_result"], &mut output.deltas);
                    }
                }
            }
            "control_request" => self.control_request(&message, &mut output),
            "result" => self.result(&message, &mut output.deltas),
            // Answers to our own control requests and bookkeeping the timeline has
            // no use for.
            "control_response" | "rate_limit_event" | "keep_alive" => {}
            _ => output.deltas.push(Delta::Unhandled(message.to_string())),
        }
        output
    }

    fn stream_event(&mut self, event: &Value, parent: Option<&str>, deltas: &mut Vec<Delta>) {
        if str_field(event, "type") != "content_block_delta" {
            return;
        }
        let delta = &event["delta"];
        let (channel, key, text) = match str_field(delta, "type") {
            "text_delta" => (TextChannel::AgentMessage, "assistant", str_field(delta, "text")),
            "thinking_delta" => (TextChannel::Reasoning, "thinking", str_field(delta, "thinking")),
            // Tool input fragments; the whole input arrives in the `assistant` message.
            _ => return,
        };
        // Thinking is often withheld: the deltas come, but empty.
        if !text.is_empty() {
            deltas.push(Delta::ItemTextDelta(TextDelta {
                key: stream(key, parent),
                channel,
                text: text.to_owned(),
            }));
        }
    }

    /// A finished content block.
    fn assistant_block(&mut self, block: &Value, parent: Option<&str>, deltas: &mut Vec<Delta>) {
        let (channel, key, text) = match str_field(block, "type") {
            "text" => (TextChannel::AgentMessage, "assistant", str_field(block, "text")),
            "thinking" => (TextChannel::Reasoning, "thinking", str_field(block, "thinking")),
            "tool_use" => {
                let id = str_field(block, "id").to_owned();
                let name = str_field(block, "name").to_owned();
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                deltas.push(Delta::ItemOpen(ItemOpen {
                    key: ItemKey::Provider(id.clone()),
                    shape: tool_shape(&name, &input, None),
                }));
                self.tools.insert(id, (name, input));
                return;
            }
            _ => return,
        };
        if !text.is_empty() {
            // The complete text settles the stream, or makes the item when nothing
            // was streamed (sub-agent messages are not).
            deltas.push(Delta::ItemTextClose(TextClose {
                key: stream(key, parent),
                channel,
                text: Some(text.to_owned()),
            }));
        }
    }

    fn tool_result(&mut self, block: &Value, details: &Value, deltas: &mut Vec<Delta>) {
        let id = str_field(block, "tool_use_id");
        // A result for a call we never saw opened carries no shape to close with.
        let Some((name, input)) = self.tools.remove(id) else {
            return;
        };
        let failed = block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let status = match (failed, self.denied.remove(id)) {
            (_, true) => ItemStatus::Declined,
            (true, false) => ItemStatus::Failed,
            (false, false) => ItemStatus::Completed,
        };
        let output = text_of(&block["content"]);
        let mut shape = tool_shape(&name, &input, Some(output));
        if let (ItemShape::FileChange(change), Some(applied)) = (&mut shape, applied_diff(details))
        {
            // The patch that was applied beats the one that was proposed.
            change.diff = Some(applied);
        }
        deltas.push(Delta::ItemClose(ItemClose {
            key: ItemKey::Provider(id.to_owned()),
            shape,
            status,
        }));
    }

    fn control_request(&mut self, message: &Value, output: &mut Output) {
        let request_id = str_field(message, "request_id").to_owned();
        let request = &message["request"];
        if str_field(request, "subtype") != "can_use_tool" {
            // Never leave a request unanswered: the agent would wait forever.
            let refusal = json!({
                "type": "control_response",
                "response": {
                    "subtype": "error",
                    "request_id": request_id,
                    "error": "qcc does not handle this request",
                },
            });
            output.writes.push(refusal.to_string());
            return;
        }
        let tool_use_id = str_field(request, "tool_use_id").to_owned();
        let name = match str_field(request, "display_name") {
            "" => str_field(request, "tool_name"),
            name => name,
        };
        let input = request.get("input").cloned().unwrap_or(Value::Null);
        // The most telling line about what is being asked.
        let detail = ["command", "file_path", "url", "pattern"]
            .iter()
            .find_map(|field| input.get(*field).and_then(Value::as_str))
            .or_else(|| request.get("description").and_then(Value::as_str))
            .map(str::to_owned);
        output.deltas.push(Delta::InteractionRequest(InteractionAsk {
            request_id: request_id.clone(),
            item: (!tool_use_id.is_empty()).then(|| ItemKey::Provider(tool_use_id.clone())),
            request: InteractionRequest::Approval(ApprovalRequest {
                title: format!("Allow {name}?"),
                detail,
                options: vec![
                    option("allow", "Allow", OptionKind::Allow),
                    option("deny", "Deny", OptionKind::Deny),
                ],
            }),
        }));
        self.approvals.insert(request_id, Approval { tool_use_id, input });
    }

    /// The end of a turn.
    fn result(&mut self, message: &Value, deltas: &mut Vec<Delta>) {
        let usage = &message["usage"];
        let tokens = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
        let cached = tokens("cache_read_input_tokens") + tokens("cache_creation_input_tokens");
        let last = TokenUsage {
            input_tokens: tokens("input_tokens"),
            output_tokens: tokens("output_tokens"),
            cached_input_tokens: Some(cached),
        };
        // Claude reports per turn; the session total is ours to keep.
        self.total.input_tokens += last.input_tokens;
        self.total.output_tokens += last.output_tokens;
        self.total.cached_input_tokens = Some(self.total.cached_input_tokens.unwrap_or(0) + cached);
        let model_context_window = message["modelUsage"]
            .as_object()
            .into_iter()
            .flat_map(|models| models.values())
            .filter_map(|model| model.get("contextWindow").and_then(Value::as_u64))
            .max();
        deltas.push(Delta::Usage(UsageSnapshot { total: self.total, last, model_context_window }));

        let failed = message.get("is_error").and_then(Value::as_bool).unwrap_or(false)
            || str_field(message, "subtype") != "success";
        if failed {
            let error = match str_field(message, "result") {
                "" => match message["errors"].as_array().filter(|errors| !errors.is_empty()) {
                    Some(errors) => {
                        errors.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")
                    }
                    None => str_field(message, "subtype").to_owned(),
                },
                text => text.to_owned(),
            };
            self.settle(TurnStatus::Failed, Some(error), deltas);
        } else {
            self.settle(TurnStatus::Completed, None, deltas);
        }
    }

    /// Close the running turn.
    fn settle(&mut self, status: TurnStatus, error: Option<String>, deltas: &mut Vec<Delta>) {
        self.tools.clear();
        self.approvals.clear();
        if !self.turn_open {
            // A result nobody asked for: let the assembler decide whether input was
            // waiting for it.
            deltas.push(Delta::TurnBoundary(TurnBoundary { status, error, claim_if_idle: true }));
            return;
        }
        self.turn_open = false;
        deltas.push(boundary(status, error));
    }
}

fn boundary(status: TurnStatus, error: Option<String>) -> Delta {
    Delta::TurnBoundary(TurnBoundary { status, error, claim_if_idle: false })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(extra: Value) -> String {
        let mut message = json!({
            "type": "result", "subtype": "success", "is_error": false,
            "usage": {"input_tokens": 2, "output_tokens": 3, "cache_read_input_tokens": 10,
                      "cache_creation_input_tokens": 5},
            "modelUsage": {"m": {"contextWindow": 1000}},
        });
        message.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        message.to_string()
    }

    fn kinds(deltas: &[Delta]) -> Vec<&'static str> {
        deltas
            .iter()
            .map(|delta| match delta {
                Delta::TurnOpen => "turn.open",
                Delta::TurnBoundary(b) => match b.status {
                    TurnStatus::Completed => "boundary.completed",
                    TurnStatus::Failed => "boundary.failed",
                    TurnStatus::Interrupted => "boundary.interrupted",
                },
                Delta::Usage(_) => "usage",
                Delta::SessionIdentity(_) => "identity",
                Delta::SessionEnded(_) => "ended",
                Delta::ProviderError(_) => "error",
                Delta::Unhandled(_) => "unhandled",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn a_steered_message_joins_the_running_turn() {
        let mut bridge = Bridge::new(None);
        assert_eq!(kinds(&bridge.prompt("one").deltas), ["turn.open"]);
        // Same wire message as a prompt, but no turn of its own.
        let steered = bridge.steer("also this");
        assert!(steered.writes[0].contains("also this"));
        assert_eq!(kinds(&steered.deltas), Vec::<&str>::new());
        // One result ends the turn both messages belong to.
        assert_eq!(
            kinds(&bridge.on_line(&result(json!({}))).deltas),
            ["usage", "boundary.completed"]
        );

        // Usage is summed across turns.
        bridge.prompt("two");
        let Delta::Usage(usage) = &bridge.on_line(&result(json!({}))).deltas[0] else {
            panic!("expected usage");
        };
        assert_eq!((usage.total.input_tokens, usage.last.input_tokens), (4, 2));
        assert_eq!(usage.total.cached_input_tokens, Some(30));
        assert_eq!(usage.model_context_window, Some(1000));
    }

    #[test]
    fn a_steer_that_missed_its_turn_becomes_a_turn_of_its_own() {
        let init = r#"{"type":"system","subtype":"init","session_id":"abc"}"#;
        let mut bridge = Bridge::new(Some("abc".into()));
        bridge.prompt("one");
        assert_eq!(bridge.on_line(init), Output::default(), "the turn we opened");
        bridge.steer("too late");
        bridge.on_line(&result(json!({})));

        // The agent starts over with the late message: its `init` opens the turn.
        assert_eq!(kinds(&bridge.on_line(init).deltas), ["turn.open"]);
        assert_eq!(
            kinds(&bridge.on_line(&result(json!({}))).deltas),
            ["usage", "boundary.completed"]
        );
    }

    #[test]
    fn file_changes_carry_a_diff() {
        let write = json!({"file_path": "a.txt", "content": "one\ntwo"});
        let edit = json!({"file_path": "a.txt", "old_string": "two", "new_string": "2\n3"});
        let multi = json!({"file_path": "a.txt", "edits": [
            {"old_string": "a", "new_string": "b"}, {"old_string": "c", "new_string": "d"}]});
        assert_eq!(proposed_diff("Write", &write).as_deref(), Some("+one\n+two"));
        assert_eq!(proposed_diff("Edit", &edit).as_deref(), Some("-two\n+2\n+3"));
        assert_eq!(proposed_diff("MultiEdit", &multi).as_deref(), Some("-a\n+b\n-c\n+d"));
        assert_eq!(proposed_diff("Read", &write), None);

        let details = json!({"structuredPatch": [
            {"oldStart": 1, "oldLines": 2, "newStart": 1, "newLines": 3, "lines": [" one", "-two", "+2", "+3"]}]});
        assert_eq!(applied_diff(&details).as_deref(), Some("@@ -1,2 +1,3 @@\n one\n-two\n+2\n+3"));
        // A new file has no hunks; the proposed diff stands.
        assert_eq!(applied_diff(&json!({"structuredPatch": []})), None);
        assert_eq!(applied_diff(&json!("Error: declined")), None);
    }

    #[test]
    fn an_interrupted_turn_is_settled_at_once_and_its_tail_is_ignored() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("count");
        let interrupt = bridge.interrupt();
        assert!(interrupt.writes[0].contains("\"subtype\":\"interrupt\""));
        assert_eq!(kinds(&interrupt.deltas), ["boundary.interrupted"]);
        assert_eq!(bridge.interrupt(), Output::default(), "nothing is running any more");

        // The next prompt opens a turn even though the old result is still to come.
        assert_eq!(kinds(&bridge.prompt("again").deltas), ["turn.open"]);
        let stale =
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"1 2 3"}]}}"#;
        assert_eq!(bridge.on_line(stale), Output::default());
        let aborted = result(json!({"subtype": "error_during_execution", "is_error": true}));
        assert_eq!(bridge.on_line(&aborted), Output::default());
        assert_eq!(
            kinds(&bridge.on_line(&result(json!({}))).deltas),
            ["usage", "boundary.completed"]
        );
    }

    #[test]
    fn failures_carry_the_agents_own_words() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("x");
        let failed =
            result(json!({"subtype": "error_max_turns", "is_error": true, "errors": ["a", "b"]}));
        let Delta::TurnBoundary(turn) = &bridge.on_line(&failed).deltas[1] else {
            panic!("expected a boundary");
        };
        assert_eq!((turn.status, turn.error.as_deref()), (TurnStatus::Failed, Some("a; b")));

        bridge.prompt("y");
        let api_error = result(json!({"is_error": true, "result": "API Error: 529"}));
        let Delta::TurnBoundary(turn) = &bridge.on_line(&api_error).deltas[1] else {
            panic!("expected a boundary");
        };
        assert_eq!(turn.error.as_deref(), Some("API Error: 529"));
    }

    #[test]
    fn session_identity_is_announced_once() {
        let init = r#"{"type":"system","subtype":"init","session_id":"abc"}"#;
        let mut fresh = Bridge::new(None);
        fresh.prompt("hello");
        assert_eq!(fresh.on_line(init).deltas, [Delta::SessionIdentity("abc".into())]);
        // Claude repeats `init` at the start of every turn.
        assert_eq!(fresh.on_line(init), Output::default());
        let mut resumed = Bridge::new(Some("abc".into()));
        resumed.prompt("hello");
        assert_eq!(resumed.on_line(init), Output::default());
    }

    #[test]
    fn process_exit_settles_the_turn_with_the_reason() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("x");
        let deltas = bridge.exited(Some(1), Some("No conversation found"));
        assert_eq!(kinds(&deltas), ["boundary.failed", "ended"]);
        assert_eq!(
            deltas[1],
            Delta::SessionEnded(Some("claude exited with code 1: No conversation found".into()))
        );
        assert_eq!(kinds(&Bridge::new(None).exited(Some(0), None)), ["ended"]);
        assert_eq!(kinds(&Bridge::new(None).exited(None, None)), ["error", "ended"]);
    }

    #[test]
    fn unknown_requests_are_refused_and_unknown_messages_kept() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("x");
        let request =
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"mcp_message"}}"#;
        let output = bridge.on_line(request);
        assert!(output.writes[0].contains("\"subtype\":\"error\""));
        assert!(output.writes[0].contains("\"request_id\":\"r1\""));
        assert_eq!(kinds(&bridge.on_line(r#"{"type":"brand_new"}"#).deltas), ["unhandled"]);
        assert_eq!(bridge.on_line("not json at all"), Output::default());
        assert!(bridge.resolve("nope", &InteractionResolution::Cancelled).is_err());
    }
}
