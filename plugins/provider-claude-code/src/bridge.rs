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
//!   of subtype `can_use_tool`, answered by a `control_response`;
//! * `AskUserQuestion` comes the same way, and the user's choices go back in its
//!   input as `answers`, keyed by the text of each question; allowing it without
//!   them tells the agent nobody answered;
//! * a `control_request` of subtype `get_context_usage` is answered with what
//!   `/context` shows, at any time and without a request to the model;
//! * the account's usage limits come in `rate_limit_event`s and in the answer to
//!   `get_usage` (see [`crate::limits`]).

use std::collections::{HashMap, HashSet};

use qcc_plugin_sdk::json::str_field;
pub use qcc_plugin_sdk::stdio::Output;
use qcc_plugin_sdk::stdio::{self, LineBridge};
use qcc_plugin_sdk::types::*;
use qcc_plugin_sdk::{Message, add_usage, base64, boundary_at, option};
use serde_json::{Value, json};

/// The prefix of our MCP status requests' ids.
const MCP_QUERY: &str = "qcc-mcp-";
/// The prefix of the ids of our requests that replace qcc's servers of the session.
const MCP_SET: &str = "qcc-set-mcp-";
/// The prefix of the ids of our context breakdown requests.
const CONTEXT_QUERY: &str = "qcc-context-";
/// Request ids of our `get_usage` requests: the account's usage limits.
const USAGE_QUERY: &str = "qcc-usage-";
/// Entries kept per part of the context: the largest, which are the ones worth
/// knowing about.
const CONTEXT_ENTRIES: usize = 20;
/// How often, and for how long, servers still connecting are asked after.
const MCP_POLL_MS: u64 = 1000;
const MCP_POLLS: u32 = 30;

/// The tool whose permission prompt is really a set of questions for the user.
const ASK_TOOL: &str = "AskUserQuestion";
/// Joins a permission request's id and the index of one of its questions into the id
/// of the interaction that asks it.
const QUESTION_SEPARATOR: char = '#';

struct Approval {
    tool_use_id: String,
    input: Value,
    /// Set for `AskUserQuestion`: its questions are asked one at a time.
    asking: Option<Asking>,
}

/// The questions of one `AskUserQuestion` call and the answers so far.
struct Asking {
    questions: Vec<Value>,
    answers: serde_json::Map<String, Value>,
}

impl Asking {
    /// The questions of a call, if it has any to ask.
    fn of(input: &Value) -> Option<Self> {
        let questions: Vec<Value> = input["questions"]
            .as_array()?
            .iter()
            .filter(|question| !str_field(question, "question").is_empty())
            .cloned()
            .collect();
        (!questions.is_empty()).then(|| Asking { questions, answers: serde_json::Map::new() })
    }

    /// The interaction that asks the next unanswered question, if any is left.
    fn next(&self, request_id: &str, tool_use_id: &str) -> Option<Delta> {
        let index = self.answers.len();
        let question = self.questions.get(index)?;
        let mut text = str_field(question, "question").to_owned();
        // A question takes one option or one line of text, so several choices are
        // typed: the agent reads the answer as written.
        if question["multiSelect"].as_bool() == Some(true) {
            text.push_str(" (several may apply: type them separated by commas)");
        }
        let options = question["options"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(at, choice)| {
                let label = str_field(choice, "label");
                (!label.is_empty()).then(|| option(&at.to_string(), label, OptionKind::Other))
            })
            .collect();
        Some(Delta::InteractionRequest(InteractionAsk {
            request_id: format!("{request_id}{QUESTION_SEPARATOR}{index}"),
            item: (!tool_use_id.is_empty()).then(|| ItemKey::Provider(tool_use_id.to_owned())),
            request: InteractionRequest::Question(QuestionRequest {
                question: text,
                options,
                allow_text: true,
            }),
        }))
    }

    /// Take the answer to the current question; `None` if it says nothing.
    fn answer(&mut self, resolution: &InteractionResolution) -> Option<()> {
        let question = self.questions.get(self.answers.len())?;
        let answer = match resolution {
            InteractionResolution::Selected(id) => {
                let at: usize = id.parse().ok()?;
                str_field(question["options"].get(at)?, "label").to_owned()
            }
            InteractionResolution::Text(text) if !text.trim().is_empty() => text.trim().to_owned(),
            _ => return None,
        };
        self.answers.insert(str_field(question, "question").to_owned(), Value::String(answer));
        Some(())
    }

    fn done(&self) -> bool {
        self.answers.len() == self.questions.len()
    }
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
    /// A read-only session: nobody is asked, whatever would be asked is refused.
    read_only: bool,
    total: TokenUsage,
    /// The main agent's latest request of the running turn. Its input is what the
    /// context holds; the turn's `result` sums every request and says nothing of that.
    last_request: Option<TokenUsage>,
    /// The model's context window as the last `result` said; only a result tells it.
    context_window: Option<u64>,
    /// The context size last reported, so that the several messages Claude sends for
    /// one request report it once.
    reported_context: Option<u64>,
    interrupts: u32,
    /// MCP status requests sent, which names them.
    mcp_queries: u32,
    /// How many times in a row the agent said a server was still connecting.
    mcp_polls: u32,
    /// Context breakdown requests sent, which names them.
    context_queries: u32,
    usage_queries: u32,
    /// The uuid of the main agent's latest message in the running turn: the entry of
    /// the session's transcript a fork from the turn keeps up to
    /// (`--resume-session-at`).
    last_message: Option<String>,
}

fn stream(channel: &str, parent: Option<&str>) -> ItemKey {
    // A sub-agent streams under its own key so it never continues the main message.
    match parent {
        Some(parent) => ItemKey::Anonymous(format!("{channel}:{parent}")),
        None => ItemKey::Anonymous(channel.to_owned()),
    }
}

/// Claude's usage block in the one dialect: what was read from the cache and what was
/// written to it both count as cached input.
fn usage_of(usage: &Value) -> TokenUsage {
    let tokens = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
    TokenUsage {
        input_tokens: tokens("input_tokens"),
        output_tokens: tokens("output_tokens"),
        cached_input_tokens: Some(
            tokens("cache_read_input_tokens") + tokens("cache_creation_input_tokens"),
        ),
    }
}

/// Tokens the context holds after a request: everything it sent, from the cache or not.
fn context_of(request: &TokenUsage) -> u64 {
    request.input_tokens + request.cached_input_tokens.unwrap_or(0)
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
        _ => ItemShape::Tool(ToolItem {
            name: name.to_owned(),
            input: input.to_string(),
            output,
            mcp_server: mcp_server(name).map(str::to_owned),
            skill: (name == "Skill")
                .then(|| str_field(input, "skill").to_owned())
                .filter(|skill| !skill.is_empty()),
        }),
    }
}

/// The server of an MCP tool, which Claude Code names `mcp__<server>__<tool>`.
fn mcp_server(name: &str) -> Option<&str> {
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    (!server.is_empty() && !tool.is_empty()).then_some(server)
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
            read_only: false,
            total: TokenUsage { input_tokens: 0, output_tokens: 0, cached_input_tokens: None },
            last_request: None,
            context_window: None,
            reported_context: None,
            interrupts: 0,
            mcp_queries: 0,
            mcp_polls: 0,
            context_queries: 0,
            usage_queries: 0,
            last_message: None,
        }
    }

    /// A read-only session. Claude's own `dontAsk` mode refuses what is not allowed
    /// up front; should it ask anyway, the bridge refuses instead of the user.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// The user sent a message that starts a turn.
    pub fn prompt(&mut self, text: &str) -> Output {
        self.prompt_message(&Message { text: text.to_owned(), images: Vec::new() })
    }

    /// [`Bridge::prompt`] with images to look at.
    pub fn prompt_message(&mut self, message: &Message) -> Output {
        let mut output = self.steer_message(message);
        if !self.turn_open {
            self.turn_open = true;
            self.last_message = None;
            output.deltas.push(Delta::TurnOpen);
        }
        output
    }

    /// Compact the session's context. Claude Code takes `/compact` as a message and
    /// runs it as a turn ending in a `result`, reporting a `compact_boundary`.
    pub fn compact(&mut self) -> Output {
        self.prompt("/compact")
    }

    /// The user sent a message for the running turn. On the wire it is the same
    /// message as a prompt: the agent takes it into the turn at its next step.
    pub fn steer(&mut self, text: &str) -> Output {
        self.steer_message(&Message { text: text.to_owned(), images: Vec::new() })
    }

    /// [`Bridge::steer`] with images to look at.
    pub fn steer_message(&mut self, message: &Message) -> Output {
        let message =
            json!({"type": "user", "message": {"role": "user", "content": content(message)}});
        Output { writes: vec![message.to_string()], ..Output::default() }
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
        // The turn's permission prompts go with it; the agent hears no to each, not
        // silence. Sorted so the agent hears them in a fixed order.
        let mut pending: Vec<&String> = self.approvals.keys().collect();
        pending.sort();
        for request_id in pending {
            output.writes.push(denial(request_id, "The turn was interrupted.").to_string());
        }
        self.draining += 1;
        self.settle(TurnStatus::Interrupted, None, &mut output.deltas);
        output
    }

    /// The user answered a permission prompt, or one question of `AskUserQuestion`.
    pub fn resolve(
        &mut self,
        request_id: &str,
        resolution: &InteractionResolution,
    ) -> Result<Output, String> {
        if let Some((permission, _)) = request_id.rsplit_once(QUESTION_SEPARATOR)
            && self.approvals.get(permission).is_some_and(|approval| approval.asking.is_some())
        {
            return Ok(self.answer_question(permission, resolution));
        }
        let approval = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| format!("no permission request {request_id:?} is pending"))?;
        let line = match resolution {
            InteractionResolution::Selected(option) if option == "allow" => {
                let response = json!({"behavior": "allow", "updatedInput": approval.input});
                success(request_id, &response)
            }
            other => {
                self.denied.insert(approval.tool_use_id);
                let message = match other {
                    InteractionResolution::Text(text) => text.as_str(),
                    _ => "The user declined this action.",
                };
                denial(request_id, message)
            }
        };
        Ok(Output { writes: vec![line.to_string()], ..Output::default() })
    }

    /// One question of `AskUserQuestion` was answered: ask the next one, or, with all
    /// of them answered, let the tool run with the answers. A question left without
    /// an answer refuses the whole call.
    fn answer_question(&mut self, request_id: &str, resolution: &InteractionResolution) -> Output {
        let mut output = Output::default();
        let Some(approval) = self.approvals.get_mut(request_id) else {
            return output;
        };
        let Some(asking) = approval.asking.as_mut() else {
            return output;
        };
        if asking.answer(resolution).is_none() {
            let approval = self.approvals.remove(request_id).expect("looked up above");
            self.denied.insert(approval.tool_use_id);
            output.writes.push(denial(request_id, "The user did not answer.").to_string());
            return output;
        }
        if let Some(next) = asking.next(request_id, &approval.tool_use_id) {
            output.deltas.push(next);
            return output;
        }
        debug_assert!(asking.done());
        let approval = self.approvals.remove(request_id).expect("looked up above");
        let mut input = approval.input;
        let answers = approval.asking.map(|asking| asking.answers).unwrap_or_default();
        if let Some(fields) = input.as_object_mut() {
            fields.insert("answers".to_owned(), Value::Object(answers));
        }
        let response = json!({"behavior": "allow", "updatedInput": input});
        output.writes.push(success(request_id, &response).to_string());
        output
    }

    /// The session was stopped on request.
    pub fn stopped(&mut self) -> Vec<Delta> {
        stdio::stopped(std::mem::take(&mut self.turn_open))
    }

    fn stream_event(&mut self, event: &Value, parent: Option<&str>, deltas: &mut Vec<Delta>) {
        // The closing count of a request, with all of its output.
        if str_field(event, "type") == "message_delta" {
            self.note_request(&event["usage"], parent, deltas);
            return;
        }
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
            output.writes.push(request_error(&request_id).to_string());
            return;
        }
        let tool_use_id = str_field(request, "tool_use_id").to_owned();
        if self.read_only {
            if !tool_use_id.is_empty() {
                self.denied.insert(tool_use_id);
            }
            let message = "This session is read-only: nothing that changes files may run, and \
                           nobody can be asked. Read files and run only the read-only commands \
                           you were allowed.";
            output.writes.push(denial(&request_id, message).to_string());
            return;
        }
        let name = match str_field(request, "display_name") {
            "" => str_field(request, "tool_name"),
            name => name,
        };
        let input = request.get("input").cloned().unwrap_or(Value::Null);
        if str_field(request, "tool_name") == ASK_TOOL
            && let Some(asking) = Asking::of(&input)
        {
            output.deltas.extend(asking.next(&request_id, &tool_use_id));
            let approval = Approval { tool_use_id, input, asking: Some(asking) };
            self.approvals.insert(request_id, approval);
            return;
        }
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
        self.approvals.insert(request_id, Approval { tool_use_id, input, asking: None });
    }

    /// Ask the agent how its MCP servers are doing. It answers before any turn, so
    /// the status is known from the start without spending one.
    pub fn query_mcp(&mut self) -> Output {
        Output { writes: vec![self.mcp_status_request()], ..Output::default() }
    }

    fn mcp_status_request(&mut self) -> String {
        self.mcp_queries += 1;
        let request_id = format!("{MCP_QUERY}{}", self.mcp_queries);
        json!({"type": "control_request", "request_id": request_id,
               "request": {"subtype": "mcp_status"}})
        .to_string()
    }

    /// Replace qcc's MCP servers of the session (those `--mcp-config` gave it, which
    /// Claude Code manages as its dynamic ones) with `servers`, then ask how the
    /// servers are. Claude Code connects the new ones, drops the missing ones and
    /// connects one with another configuration again; the servers of the user's own
    /// configuration stay. It takes the request mid-turn as well.
    pub fn set_mcp_servers(&mut self, servers: &[McpServer]) -> Output {
        self.mcp_queries += 1;
        let request_id = format!("{MCP_SET}{}", self.mcp_queries);
        let set = json!({"type": "control_request", "request_id": request_id,
            "request": {"subtype": "mcp_set_servers", "servers": mcp_servers_json(servers)}});
        self.mcp_polls = 0;
        Output { writes: vec![set.to_string(), self.mcp_status_request()], ..Output::default() }
    }

    /// Ask the agent what its context window holds. `summary` answers from the
    /// last request's usage and local counts, without asking the model anything.
    pub fn query_context(&mut self) -> Output {
        Output { writes: vec![self.context_request()], ..Output::default() }
    }

    fn context_request(&mut self) -> String {
        self.context_queries += 1;
        let request_id = format!("{CONTEXT_QUERY}{}", self.context_queries);
        json!({"type": "control_request", "request_id": request_id,
               "request": {"subtype": "get_context_usage", "detail": "summary"}})
        .to_string()
    }

    /// Ask how much of the account's usage limits is spent. Answered from the
    /// usage endpoint, without a turn; `rate_limit_event`s keep it current after.
    pub fn query_limits(&mut self) -> Output {
        self.usage_queries += 1;
        let request = crate::limits::request(&format!("{USAGE_QUERY}{}", self.usage_queries));
        Output { writes: vec![request], ..Output::default() }
    }

    fn control_response(&mut self, response: &Value, output: &mut Output) {
        if str_field(response, "request_id").starts_with(USAGE_QUERY) {
            // An agent too old to tell answers with an error, which is ignored.
            if str_field(response, "subtype") == "success" {
                let limits = crate::limits::from_usage(&response["response"]);
                output.deltas.push(Delta::RateLimits(limits));
            }
            return;
        }
        if str_field(response, "request_id").starts_with(CONTEXT_QUERY) {
            // An agent too old to tell answers with an error, which is ignored.
            if str_field(response, "subtype") == "success"
                && let Some(context) = context_usage(&response["response"])
            {
                output.deltas.push(Delta::ContextUsage(context));
            }
            return;
        }
        if str_field(response, "request_id").starts_with(MCP_SET) {
            self.mcp_servers_set(response, output);
            return;
        }
        // An agent too old to tell answers with an error, which is ignored.
        if !str_field(response, "request_id").starts_with(MCP_QUERY)
            || str_field(response, "subtype") != "success"
        {
            return;
        }
        let servers = response["response"]["mcpServers"].as_array().into_iter().flatten();
        let mut connecting = false;
        for server in servers {
            let state = match str_field(server, "status") {
                "pending" => McpState::Starting,
                "connected" => McpState::Ready,
                "failed" => McpState::Failed,
                "needs-auth" => McpState::NeedsAuth,
                "disabled" => McpState::Disabled,
                _ => continue,
            };
            connecting |= state == McpState::Starting;
            let error = server.get("error").and_then(Value::as_str).map(str::to_owned);
            output.deltas.push(Delta::McpStatus(McpStatus {
                name: str_field(server, "name").to_owned(),
                state,
                error,
            }));
        }
        // Servers connect in the background and nothing says when they are done.
        if connecting && self.mcp_polls < MCP_POLLS {
            self.mcp_polls += 1;
            output.wake_after_ms = Some(MCP_POLL_MS);
        }
    }

    /// Claude Code's answer to `mcp_set_servers`: the servers it dropped leave the
    /// list. Those it could not connect are reported failed by the status query
    /// sent along.
    fn mcp_servers_set(&mut self, response: &Value, output: &mut Output) {
        if str_field(response, "subtype") != "success" {
            let reason = str_field(response, "error");
            output.deltas.push(Delta::ProviderWarning(format!(
                "Claude Code could not change the MCP servers of the session: {reason}"
            )));
            return;
        }
        let removed = response["response"]["removed"].as_array().into_iter().flatten();
        output.deltas.extend(
            removed.filter_map(Value::as_str).map(|name| Delta::McpRemoved(name.to_owned())),
        );
    }

    /// Remember the usage of one of the main agent's requests and report how full the
    /// context is now, without waiting for the turn to end. A sub-agent's requests
    /// carry a context of their own, so they say nothing of how full this one is.
    fn note_request(&mut self, usage: &Value, parent: Option<&str>, deltas: &mut Vec<Delta>) {
        if parent.is_some() || !usage.is_object() {
            return;
        }
        let last = usage_of(usage);
        self.last_request = Some(last);
        let context_used = Some(context_of(&last));
        if context_used == self.reported_context {
            return;
        }
        self.reported_context = context_used;
        // The total stays as it was: what the turn spent is added once, by its result.
        deltas.push(Delta::Usage(UsageSnapshot {
            total: self.total,
            last,
            model_context_window: self.context_window,
            context_used,
        }));
    }

    /// The end of a turn.
    fn result(&mut self, message: &Value, deltas: &mut Vec<Delta>) {
        // Claude reports per turn, summed over every request of it; the session total
        // is ours to keep.
        let turn = usage_of(&message["usage"]);
        add_usage(&mut self.total, &turn);
        // Without a request seen (a turn that failed before one), the turn is the best
        // there is.
        let last = self.last_request.take().unwrap_or(turn);
        let model_context_window = message["modelUsage"]
            .as_object()
            .into_iter()
            .flat_map(|models| models.values())
            .filter_map(|model| model.get("contextWindow").and_then(Value::as_u64))
            .max();
        if model_context_window.is_some() {
            self.context_window = model_context_window;
        }
        let context_used = Some(context_of(&last));
        self.reported_context = context_used;
        deltas.push(Delta::Usage(UsageSnapshot {
            total: self.total,
            last,
            model_context_window,
            context_used,
        }));

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
        // Where a fork from this turn starts: the main agent's last message in it.
        let point = self.last_message.take();
        if !self.turn_open {
            // A result nobody asked for: let the assembler decide whether input was
            // waiting for it.
            deltas.push(Delta::TurnBoundary(TurnBoundary {
                status,
                error,
                claim_if_idle: true,
                point,
            }));
            return;
        }
        self.turn_open = false;
        deltas.push(boundary_at(status, error, point));
    }
}

/// The answer to a control request of the agent.
fn success(request_id: &str, response: &Value) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": request_id, "response": response},
    })
}

/// The content of a user message: plain text, or image blocks followed by the text
/// when there are images to look at.
fn content(message: &Message) -> Value {
    if message.images.is_empty() {
        return Value::String(message.text.clone());
    }
    let mut blocks: Vec<Value> = message
        .images
        .iter()
        .filter_map(|image| {
            let data = image.data.as_deref()?;
            Some(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": image.mime, "data": base64(data)},
            }))
        })
        .collect();
    blocks.push(json!({"type": "text", "text": message.text}));
    Value::Array(blocks)
}

/// No to a permission prompt.
fn denial(request_id: &str, message: &str) -> Value {
    success(request_id, &json!({"behavior": "deny", "message": message}))
}

/// The answer to a control request qcc does not handle.
fn request_error(request_id: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "error",
            "request_id": request_id,
            "error": "qcc does not handle this request",
        },
    })
}

impl LineBridge for Bridge {
    /// One line of the agent's stdout.
    fn on_line(&mut self, line: &str) -> Output {
        let mut output = Output::default();
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return output;
        };
        let kind = str_field(&message, "type");

        if kind == "system" {
            // The agent summed up the conversation: when asked (`/compact`) or on
            // its own, its context being full.
            if str_field(&message, "subtype") == "compact_boundary" {
                let metadata = &message["compact_metadata"];
                output.deltas.push(Delta::ContextCompacted(Compaction {
                    automatic: str_field(metadata, "trigger") == "auto",
                    tokens_before: metadata["pre_tokens"].as_u64(),
                    tokens_after: metadata["post_tokens"].as_u64(),
                }));
            }
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
        // Answers to our own requests come whatever the turn is doing.
        if kind == "control_response" {
            self.control_response(&message["response"], &mut output);
            return output;
        }
        // The account's limits, whatever the turn is doing.
        if kind == "rate_limit_event" {
            if let Some(limits) = crate::limits::from_event(&message["rate_limit_info"]) {
                output.deltas.push(Delta::RateLimits(limits));
            }
            return output;
        }
        if self.draining > 0 {
            match kind {
                "result" => self.draining -= 1,
                // The turn is over for us, but the agent still waits for an answer.
                "control_request" => {
                    let request_id = str_field(&message, "request_id");
                    let refusal = match str_field(&message["request"], "subtype") {
                        "can_use_tool" => denial(request_id, "The turn was interrupted."),
                        _ => request_error(request_id),
                    };
                    output.writes.push(refusal.to_string());
                }
                _ => {}
            }
            return output;
        }

        let parent = message.get("parent_tool_use_id").and_then(Value::as_str);
        match kind {
            "stream_event" => self.stream_event(&message["event"], parent, &mut output.deltas),
            "assistant" => {
                // A sub-agent's messages are a transcript of their own.
                if parent.is_none()
                    && let Some(uuid) = message.get("uuid").and_then(Value::as_str)
                {
                    self.last_message = Some(uuid.to_owned());
                }
                self.note_request(&message["message"]["usage"], parent, &mut output.deltas);
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
            "result" => {
                self.result(&message, &mut output.deltas);
                // A turn may have started, lost or signed in to a server.
                self.mcp_polls = 0;
                output.writes.push(self.mcp_status_request());
                // The turn filled the context; `/compact` emptied it.
                output.writes.push(self.context_request());
            }
            // Bookkeeping the timeline has no use for.
            "keep_alive" => {}
            _ => output.deltas.push(Delta::Unhandled(message.to_string())),
        }
        output
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta> {
        let turn_open = std::mem::take(&mut self.turn_open);
        stdio::exited("claude", turn_open, code != Some(0), code, stderr)
    }

    fn on_wake(&mut self) -> Output {
        self.query_mcp()
    }
}

/// The answer to `get_context_usage` in the one dialect, or nothing if it is not one.
fn context_usage(response: &Value) -> Option<ContextUsage> {
    let categories: Vec<ContextCategory> = response["categories"]
        .as_array()?
        .iter()
        .filter_map(|category| {
            let name = category["name"].as_str()?.to_owned();
            let id = match category["id"].as_str() {
                Some(id) => id.to_owned(),
                None => name.to_lowercase().replace(' ', "_"),
            };
            let kind = match category["kind"].as_str() {
                Some("free") => ContextKind::Free,
                Some("buffer") => ContextKind::Buffer,
                Some("deferred") => ContextKind::Deferred,
                Some(_) => ContextKind::Used,
                // Older agents do not say; their ids and flags do.
                None if id == "free_space" => ContextKind::Free,
                None if id.ends_with("_buffer") => ContextKind::Buffer,
                None if category["isDeferred"] == true => ContextKind::Deferred,
                None => ContextKind::Used,
            };
            Some(ContextCategory { id, name, tokens: category["tokens"].as_u64()?, kind })
        })
        .collect();
    let used = categories
        .iter()
        .filter(|category| category.kind == ContextKind::Used)
        .map(|category| category.tokens)
        .sum();
    let mut entries = Vec::new();
    let mut part = |category: &str, list: &Value, name: &dyn Fn(&Value) -> Option<String>| {
        let mut found: Vec<ContextEntry> = list
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| {
                Some(ContextEntry {
                    category: category.to_owned(),
                    name: name(entry)?,
                    tokens: entry["tokens"].as_u64()?,
                })
            })
            .filter(|entry| entry.tokens > 0)
            .collect();
        found.sort_by(|a, b| b.tokens.cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
        entries.extend(found.into_iter().take(CONTEXT_ENTRIES));
    };
    let field = |name: &'static str| move |entry: &Value| entry[name].as_str().map(str::to_owned);
    part("system_tools", &response["systemTools"], &field("name"));
    part("memory_files", &response["memoryFiles"], &field("path"));
    part("skills", &response["skills"]["skillFrontmatter"], &field("name"));
    part("custom_agents", &response["agents"], &field("agentType"));
    // Loaded MCP tools are in the window; the others wait, deferred, to be looked up.
    let (loaded, deferred): (Vec<Value>, Vec<Value>) = response["mcpTools"]
        .as_array()
        .into_iter()
        .flatten()
        .cloned()
        .partition(|tool| tool["isLoaded"] != false);
    let mcp_tool = |tool: &Value| {
        let name = tool["name"].as_str()?;
        // `mcp__server__tool`: the server is said apart.
        let short = name.rsplit("__").next().unwrap_or(name);
        Some(match tool["serverName"].as_str() {
            Some(server) => format!("{server}: {short}"),
            None => name.to_owned(),
        })
    };
    part("mcp_tools", &Value::Array(loaded), &mcp_tool);
    part("mcp_tools_deferred", &Value::Array(deferred), &mcp_tool);
    let messages = &response["messageBreakdown"];
    let message_parts: Vec<Value> = [
        ("Tool calls", "toolCallTokens"),
        ("Tool results", "toolResultTokens"),
        ("Attachments", "attachmentTokens"),
        ("Agent messages", "assistantMessageTokens"),
        ("Your messages", "userMessageTokens"),
    ]
    .iter()
    .map(|(name, field)| json!({"name": name, "tokens": messages[*field]}))
    .collect();
    part("messages", &Value::Array(message_parts), &field("name"));
    Some(ContextUsage {
        used,
        window: response["maxTokens"].as_u64().filter(|window| *window > 0),
        estimated: false,
        categories,
        entries,
    })
}

/// The value of `--mcp-config` that adds qcc's MCP servers to the user's own, or
/// nothing when there are none.
pub fn mcp_config(servers: &[McpServer]) -> Option<String> {
    if servers.is_empty() {
        return None;
    }
    Some(json!({ "mcpServers": mcp_servers_json(servers) }).to_string())
}

/// qcc's MCP servers as Claude Code configures them: an object by name.
fn mcp_servers_json(servers: &[McpServer]) -> Value {
    let servers: serde_json::Map<String, Value> = servers
        .iter()
        .map(|server| {
            let config = match &server.transport {
                McpTransport::Stdio(stdio) => json!({
                    "type": "stdio",
                    "command": stdio.command,
                    "args": stdio.args,
                    "env": pairs(&stdio.env),
                }),
                McpTransport::Http(http) => json!({
                    "type": "http",
                    "url": http.url,
                    "headers": pairs(&http.headers),
                }),
            };
            (server.name.clone(), config)
        })
        .collect();
    Value::Object(servers)
}

fn pairs(pairs: &[(String, String)]) -> serde_json::Map<String, Value> {
    pairs.iter().map(|(name, value)| (name.clone(), json!(value))).collect()
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
    fn compacting_is_a_turn_that_reports_the_boundary() {
        let mut bridge = Bridge::new(None);
        let asked = bridge.compact();
        assert!(asked.writes[0].contains("\"content\":\"/compact\""), "{:?}", asked.writes);
        assert_eq!(kinds(&asked.deltas), ["turn.open"]);
        let boundary = json!({"type": "system", "subtype": "compact_boundary",
            "compact_metadata": {"trigger": "manual", "pre_tokens": 900, "post_tokens": 100}});
        assert_eq!(
            bridge.on_line(&boundary.to_string()).deltas,
            [Delta::ContextCompacted(Compaction {
                automatic: false,
                tokens_before: Some(900),
                tokens_after: Some(100),
            })]
        );
        // The summary is replayed as a user message with text content: not a tool result.
        let replay = json!({"type": "user", "message": {"role": "user", "content": "Summary"}});
        assert!(bridge.on_line(&replay.to_string()).deltas.is_empty());
        assert_eq!(
            kinds(&bridge.on_line(&result(json!({}))).deltas),
            ["usage", "boundary.completed"]
        );

        // On its own, mid-turn, the agent says so too.
        bridge.prompt("go on");
        let auto = json!({"type": "system", "subtype": "compact_boundary",
            "compact_metadata": {"trigger": "auto"}});
        let Delta::ContextCompacted(compaction) = &bridge.on_line(&auto.to_string()).deltas[0]
        else {
            panic!("expected a compaction");
        };
        assert!(compaction.automatic);
    }

    #[test]
    fn a_turn_ends_at_the_main_agent_s_last_message() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("one");
        let assistant = |uuid: &str, parent: Value| {
            json!({"type": "assistant", "uuid": uuid, "parent_tool_use_id": parent,
                   "message": {"content": []}})
            .to_string()
        };
        bridge.on_line(&assistant("a1", Value::Null));
        bridge.on_line(&assistant("a2", Value::Null));
        // A sub-agent's message is not in the main transcript.
        bridge.on_line(&assistant("sub", json!("toolu_1")));
        let point = |deltas: &[Delta]| {
            deltas.iter().find_map(|delta| match delta {
                Delta::TurnBoundary(boundary) => Some(boundary.point.clone()),
                _ => None,
            })
        };
        assert_eq!(point(&bridge.on_line(&result(json!({}))).deltas), Some(Some("a2".into())));
        // A turn in which the agent said nothing has no point.
        bridge.prompt("two");
        assert_eq!(point(&bridge.on_line(&result(json!({}))).deltas), Some(None));
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
    fn the_context_is_the_last_request_not_the_whole_turn() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("go");
        let request = |cached: u64, parent: Value| {
            json!({
                "type": "assistant", "parent_tool_use_id": parent,
                "message": {"content": [], "usage": {"input_tokens": 1, "output_tokens": 7,
                    "cache_read_input_tokens": cached, "cache_creation_input_tokens": 0}},
            })
            .to_string()
        };
        bridge.on_line(&request(100, Value::Null));
        bridge.on_line(&request(200, Value::Null));
        // A sub-agent's request has a context of its own.
        bridge.on_line(&request(9, json!("toolu_1")));
        // The result sums all requests: 100 + 200 + 9 cached, and more.
        let Delta::Usage(usage) = &bridge.on_line(&result(json!({}))).deltas[0] else {
            panic!("expected usage");
        };
        assert_eq!(usage.last.cached_input_tokens, Some(200));
        assert_eq!((usage.last.input_tokens, usage.last.output_tokens), (1, 7));
        assert_eq!(usage.total.cached_input_tokens, Some(15));

        // The next turn starts without a request of its own seen: the result stands in.
        bridge.prompt("again");
        let Delta::Usage(usage) = &bridge.on_line(&result(json!({}))).deltas[0] else {
            panic!("expected usage");
        };
        assert_eq!(usage.last.cached_input_tokens, Some(15));
    }

    #[test]
    fn the_context_is_reported_after_every_request_of_the_turn() {
        let mut bridge = Bridge::new(None);
        let nothing = TokenUsage { input_tokens: 0, output_tokens: 0, cached_input_tokens: None };
        let usage = |cached: u64| {
            json!({"input_tokens": 1, "output_tokens": 7,
                   "cache_read_input_tokens": cached, "cache_creation_input_tokens": 3})
        };
        let assistant = |cached: u64, parent: Value| {
            json!({"type": "assistant", "parent_tool_use_id": parent,
                   "message": {"content": [], "usage": usage(cached)}})
            .to_string()
        };
        let closing = |cached: u64| {
            json!({"type": "stream_event", "parent_tool_use_id": null,
                   "event": {"type": "message_delta", "usage": usage(cached)}})
            .to_string()
        };
        let snapshots = |bridge: &mut Bridge, line: &str| -> Vec<UsageSnapshot> {
            let deltas = bridge.on_line(line).deltas;
            deltas
                .into_iter()
                .filter_map(|delta| match delta {
                    Delta::Usage(usage) => Some(usage),
                    _ => None,
                })
                .collect()
        };

        bridge.prompt("go");
        let first = snapshots(&mut bridge, &assistant(100, Value::Null));
        assert_eq!(first.len(), 1, "{first:?}");
        assert_eq!(first[0].context_used, Some(104));
        assert_eq!(first[0].total, nothing, "nothing is spent before a result");
        assert_eq!(first[0].model_context_window, None, "no result has told it yet");
        // More messages of the same request, and its closing count, say nothing new.
        assert!(snapshots(&mut bridge, &assistant(100, Value::Null)).is_empty());
        assert!(snapshots(&mut bridge, &closing(100)).is_empty());
        // A sub-agent's request has a context of its own.
        assert!(snapshots(&mut bridge, &assistant(9, json!("toolu_1"))).is_empty());
        let second = snapshots(&mut bridge, &closing(200));
        assert_eq!(second.len(), 1, "{second:?}");
        assert_eq!(second[0].context_used, Some(204));
        assert_eq!(second[0].total, nothing);

        let end = snapshots(&mut bridge, &result(json!({})));
        assert_eq!(end.len(), 1, "{end:?}");
        assert_eq!(end[0].context_used, Some(204));
        assert_eq!(end[0].total.input_tokens, 2);
        assert_eq!(end[0].total.cached_input_tokens, Some(15));
        assert_eq!(end[0].model_context_window, Some(1000));

        // The next turn's requests carry the session total and the window learned so far.
        bridge.prompt("again");
        let next = snapshots(&mut bridge, &assistant(300, Value::Null));
        assert_eq!(next.len(), 1, "{next:?}");
        assert_eq!(next[0].total, end[0].total);
        assert_eq!(next[0].model_context_window, Some(1000));
        assert_eq!(next[0].context_used, Some(304));
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
    fn an_interrupt_answers_every_request_of_the_turn() {
        let ask = |id: &str| {
            json!({"type": "control_request", "request_id": id,
                   "request": {"subtype": "can_use_tool", "tool_name": "Bash",
                               "tool_use_id": format!("tu-{id}"), "input": {"command": "ls"}}})
            .to_string()
        };
        let mut bridge = Bridge::new(None);
        bridge.prompt("go");
        bridge.on_line(&ask("r1"));
        let interrupt = bridge.interrupt();
        assert_eq!(interrupt.writes.len(), 2, "{:?}", interrupt.writes);
        assert!(interrupt.writes[1].contains("\"request_id\":\"r1\""));
        assert!(interrupt.writes[1].contains("\"behavior\":\"deny\""));

        // A prompt that raced the interrupt is refused too, and shown to nobody.
        let late = bridge.on_line(&ask("r2"));
        assert_eq!(late.deltas, []);
        assert!(late.writes[0].contains("\"request_id\":\"r2\""), "{late:?}");
        assert!(late.writes[0].contains("\"behavior\":\"deny\""));
    }

    #[test]
    fn a_read_only_session_refuses_what_would_be_asked_and_asks_nobody() {
        let ask = json!({"type": "control_request", "request_id": "r1",
                         "request": {"subtype": "can_use_tool", "tool_name": "Write",
                                     "tool_use_id": "tu-1", "input": {"file_path": "/w/a"}}})
        .to_string();
        let mut bridge = Bridge::new(None).read_only();
        bridge.prompt("go");
        let output = bridge.on_line(&ask);
        assert!(output.deltas.is_empty(), "nobody is asked: {:?}", output.deltas);
        assert!(output.writes[0].contains("\"behavior\":\"deny\""), "{output:?}");
        assert!(output.writes[0].contains("read-only"), "{output:?}");
    }

    #[test]
    fn ask_user_question_is_asked_question_by_question_and_answered_in_the_input() {
        let input = json!({"questions": [
            {"question": "Which language?", "header": "Lang", "multiSelect": false,
             "options": [{"label": "English", "description": "e"},
                         {"label": "Russian", "description": "r"}]},
            {"question": "Which parts?", "header": "Parts", "multiSelect": true,
             "options": [{"label": "A", "description": "a"}, {"label": "B", "description": "b"}]},
        ]});
        let ask = json!({"type": "control_request", "request_id": "r1",
                         "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                                     "tool_use_id": "tu-1", "input": input}})
        .to_string();
        let question = |deltas: &[Delta]| match deltas {
            [Delta::InteractionRequest(ask)] => match &ask.request {
                InteractionRequest::Question(question) => {
                    (ask.request_id.clone(), question.clone())
                }
                other => panic!("expected a question, got {other:?}"),
            },
            other => panic!("expected one interaction, got {other:?}"),
        };
        let mut bridge = Bridge::new(None);
        bridge.prompt("go");

        let (id, first) = question(&bridge.on_line(&ask).deltas);
        assert_eq!(id, "r1#0");
        assert_eq!(first.question, "Which language?");
        let labels: Vec<&str> = first.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["English", "Russian"]);
        assert!(first.allow_text);

        // The first answer asks the second question and tells the agent nothing yet.
        let next = bridge.resolve(&id, &InteractionResolution::Selected("1".into())).unwrap();
        assert_eq!(next.writes, Vec::<String>::new());
        let (id, second) = question(&next.deltas);
        assert_eq!(id, "r1#1");
        assert!(second.question.starts_with("Which parts? (several"), "{}", second.question);

        let last = bridge.resolve(&id, &InteractionResolution::Text("A, B".into())).unwrap();
        assert_eq!(last.deltas, []);
        let reply: Value = serde_json::from_str(&last.writes[0]).unwrap();
        let response = &reply["response"]["response"];
        assert_eq!(reply["response"]["request_id"], "r1");
        assert_eq!(response["behavior"], "allow");
        assert_eq!(
            response["updatedInput"]["answers"],
            json!({"Which language?": "Russian", "Which parts?": "A, B"})
        );
        assert_eq!(response["updatedInput"]["questions"], input["questions"]);
    }

    #[test]
    fn an_unanswered_question_refuses_the_call_and_an_interrupt_refuses_it_too() {
        let ask = |id: &str| {
            json!({"type": "control_request", "request_id": id,
                   "request": {"subtype": "can_use_tool", "tool_name": "AskUserQuestion",
                               "tool_use_id": format!("tu-{id}"),
                               "input": {"questions": [{"question": "Go?", "options": []}]}}})
            .to_string()
        };
        let mut bridge = Bridge::new(None);
        bridge.prompt("go");
        bridge.on_line(&ask("r1"));
        let refused = bridge.resolve("r1#0", &InteractionResolution::Cancelled).unwrap();
        assert!(refused.writes[0].contains("\"behavior\":\"deny\""), "{refused:?}");
        assert!(bridge.resolve("r1#0", &InteractionResolution::Text("y".into())).is_err());

        bridge.on_line(&ask("r2"));
        let interrupt = bridge.interrupt();
        assert!(interrupt.writes[1].contains("\"request_id\":\"r2\""), "{interrupt:?}");
        assert!(interrupt.writes[1].contains("\"behavior\":\"deny\""));
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

    #[test]
    fn mcp_servers_become_one_mcp_config() {
        assert_eq!(mcp_config(&[]), None);
        let servers = [
            McpServer {
                name: "docs".into(),
                transport: McpTransport::Stdio(McpStdio {
                    command: "npx".into(),
                    args: vec!["docs-mcp".into()],
                    env: vec![("TOKEN".into(), "t".into())],
                }),
            },
            McpServer {
                name: "web".into(),
                transport: McpTransport::Http(McpHttp {
                    url: "https://example.com/mcp".into(),
                    headers: vec![("Authorization".into(), "Bearer x".into())],
                }),
            },
        ];
        let config: Value = serde_json::from_str(&mcp_config(&servers).unwrap()).unwrap();
        assert_eq!(
            config,
            json!({"mcpServers": {
                "docs": {"type": "stdio", "command": "npx", "args": ["docs-mcp"],
                         "env": {"TOKEN": "t"}},
                "web": {"type": "http", "url": "https://example.com/mcp",
                        "headers": {"Authorization": "Bearer x"}},
            }})
        );
    }

    #[test]
    fn mcp_status_is_asked_for_until_the_servers_settle() {
        let mut bridge = Bridge::new(None);
        let query = bridge.query_mcp();
        let request: Value = serde_json::from_str(&query.writes[0]).unwrap();
        assert_eq!(request["request"]["subtype"], "mcp_status");
        let id = request["request_id"].as_str().unwrap().to_owned();

        let answer = |id: &str, servers: Value| {
            json!({"type": "control_response", "response": {"subtype": "success",
                   "request_id": id, "response": {"mcpServers": servers}}})
            .to_string()
        };
        let pending = bridge.on_line(&answer(
            &id,
            json!([{"name": "docs", "status": "pending"},
                   {"name": "off", "status": "disabled"},
                   {"name": "web", "status": "failed", "error": "ENOENT"},
                   {"name": "odd", "status": "brand-new"}]),
        ));
        assert_eq!(
            pending.deltas,
            [
                Delta::McpStatus(McpStatus {
                    name: "docs".into(),
                    state: McpState::Starting,
                    error: None,
                }),
                Delta::McpStatus(McpStatus {
                    name: "off".into(),
                    state: McpState::Disabled,
                    error: None,
                }),
                Delta::McpStatus(McpStatus {
                    name: "web".into(),
                    state: McpState::Failed,
                    error: Some("ENOENT".into()),
                }),
            ]
        );
        assert_eq!(pending.wake_after_ms, Some(MCP_POLL_MS));
        assert_eq!(LineBridge::on_wake(&mut bridge).writes.len(), 1);

        let ready = bridge.on_line(&answer(&id, json!([{"name": "docs", "status": "connected"}])));
        assert_eq!(ready.wake_after_ms, None);
        // Not ours, or an agent that cannot tell: nothing to report.
        assert_eq!(
            bridge.on_line(&answer("r9", json!([{"name": "x", "status": "connected"}]))),
            Output::default()
        );
        let refused = r#"{"type":"control_response","response":{"subtype":"error","request_id":"qcc-mcp-3","error":"unknown"}}"#;
        assert_eq!(bridge.on_line(refused), Output::default());

        // After every turn it is asked again.
        bridge.prompt("x");
        let done = bridge.on_line(&result(json!({})));
        assert!(done.writes.iter().any(|line| line.contains("mcp_status")));
    }

    #[test]
    fn mcp_servers_are_replaced_in_the_running_session() {
        let mut bridge = Bridge::new(None);
        let docs = McpServer {
            name: "docs".into(),
            transport: McpTransport::Stdio(McpStdio {
                command: "npx".into(),
                args: vec!["docs-mcp".into()],
                env: Vec::new(),
            }),
        };
        let set = bridge.set_mcp_servers(&[docs]);
        let requests: Vec<Value> =
            set.writes.iter().map(|line| serde_json::from_str(line).unwrap()).collect();
        assert_eq!(
            requests[0]["request"],
            json!({"subtype": "mcp_set_servers", "servers": {
                "docs": {"type": "stdio", "command": "npx", "args": ["docs-mcp"], "env": {}}}})
        );
        assert_eq!(requests[1]["request"]["subtype"], "mcp_status");

        // What Claude Code dropped leaves the list.
        let id = requests[0]["request_id"].as_str().unwrap();
        let done = json!({"type": "control_response", "response": {"subtype": "success",
            "request_id": id, "response": {"added": ["docs"], "removed": ["web"], "errors": {}}}});
        assert_eq!(bridge.on_line(&done.to_string()).deltas, [Delta::McpRemoved("web".into())]);

        let again = bridge.set_mcp_servers(&[]);
        let request: Value = serde_json::from_str(&again.writes[0]).unwrap();
        assert_eq!(request["request"]["servers"], json!({}));
        let refused = json!({"type": "control_response", "response": {"subtype": "error",
            "request_id": request["request_id"], "error": "unknown subtype"}});
        let warned = bridge.on_line(&refused.to_string());
        assert!(
            matches!(&warned.deltas[..], [Delta::ProviderWarning(w)] if w.contains("unknown subtype")),
            "{warned:?}"
        );
    }

    #[test]
    fn the_context_is_asked_after_every_turn_and_read_part_by_part() {
        let mut bridge = Bridge::new(None);
        bridge.prompt("hi");
        let done = bridge.on_line(&result(json!({})));
        let request = done
            .writes
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|request| request["request"]["subtype"] == "get_context_usage")
            .expect("a context request");
        assert_eq!(request["request"]["detail"], "summary");

        // Trimmed from what Claude Code 2.1.325 answered.
        let answer = json!({"type": "control_response", "response": {"subtype": "success",
            "request_id": request["request_id"], "response": {
            "categories": [
                {"id": "system_prompt", "name": "System prompt", "tokens": 3506, "kind": "used"},
                {"id": "system_tools_deferred", "name": "System tools (deferred)",
                 "tokens": 15338, "isDeferred": true, "kind": "deferred"},
                {"id": "skills", "name": "Skills", "tokens": 5826, "kind": "used"},
                {"name": "Messages", "tokens": 120},
                {"id": "autocompact_buffer", "name": "Autocompact buffer", "tokens": 33000,
                 "kind": "buffer"},
                {"id": "free_space", "name": "Free space", "tokens": 942548, "kind": "free"}],
            "totalTokens": 9452, "maxTokens": 1000000,
            "memoryFiles": [{"path": "/w/CLAUDE.md", "type": "Project", "tokens": 40}],
            "mcpTools": [
                {"name": "mcp__docs__search", "serverName": "docs", "tokens": 90, "isLoaded": true},
                {"name": "mcp__docs__fetch", "serverName": "docs", "tokens": 70, "isLoaded": false}],
            "skills": {"totalSkills": 2, "includedSkills": 2, "tokens": 5826, "skillFrontmatter": [
                {"name": "tdd", "source": "userSettings", "tokens": 23},
                {"name": "docs", "source": "userSettings", "tokens": 33}]},
            "messageBreakdown": {"toolCallTokens": 0, "toolResultTokens": 80,
                "attachmentTokens": 0, "assistantMessageTokens": 30, "userMessageTokens": 10}
        }}});
        let [Delta::ContextUsage(context)] = &bridge.on_line(&answer.to_string()).deltas[..] else {
            panic!("expected a context breakdown");
        };
        assert_eq!((context.used, context.window, context.estimated), (9452, Some(1000000), false));
        let kinds: Vec<_> = context.categories.iter().map(|c| (c.id.as_str(), c.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("system_prompt", ContextKind::Used),
                ("system_tools_deferred", ContextKind::Deferred),
                ("skills", ContextKind::Used),
                ("messages", ContextKind::Used),
                ("autocompact_buffer", ContextKind::Buffer),
                ("free_space", ContextKind::Free),
            ]
        );
        let entries: Vec<_> = context
            .entries
            .iter()
            .map(|e| (e.category.as_str(), e.name.as_str(), e.tokens))
            .collect();
        assert_eq!(
            entries,
            [
                ("memory_files", "/w/CLAUDE.md", 40),
                ("skills", "docs", 33),
                ("skills", "tdd", 23),
                ("mcp_tools", "docs: search", 90),
                ("mcp_tools_deferred", "docs: fetch", 70),
                ("messages", "Tool results", 80),
                ("messages", "Agent messages", 30),
                ("messages", "Your messages", 10),
            ]
        );

        // An agent that cannot tell says nothing.
        let refused = json!({"type": "control_response", "response": {"subtype": "error",
            "request_id": "qcc-context-9", "error": "unknown subtype"}});
        assert!(bridge.on_line(&refused.to_string()).deltas.is_empty());
    }

    #[test]
    fn tools_of_mcp_servers_and_skills_are_said_apart() {
        let tool = |name: &str, input: Value| match tool_shape(name, &input, None) {
            ItemShape::Tool(tool) => (tool.mcp_server, tool.skill),
            other => panic!("not a tool: {other:?}"),
        };
        assert_eq!(tool("mcp__docs__search", json!({})), (Some("docs".into()), None));
        assert_eq!(tool("Skill", json!({"skill": "tdd"})), (None, Some("tdd".into())));
        assert_eq!(tool("mcp__broken", json!({})), (None, None));
        assert_eq!(tool("TodoWrite", json!({})), (None, None));
    }

    fn attachment(name: &str, mime: &str, data: Option<&[u8]>) -> Attachment {
        Attachment {
            name: name.into(),
            mime: mime.into(),
            path: format!("/data/attachments/{name}"),
            size: 3,
            data: data.map(<[u8]>::to_vec),
        }
    }

    #[test]
    fn images_go_as_image_blocks_and_other_files_by_path() {
        let attachments = [
            attachment("shot.png", "image/png", Some(b"abc")),
            attachment("a.txt", "text/plain", None),
        ];
        let message = qcc_plugin_sdk::message("look", &[], &attachments, true);
        let mut bridge = Bridge::new(None);
        let output = bridge.prompt_message(&message);
        assert_eq!(kinds(&output.deltas), ["turn.open"]);
        let written: Value = serde_json::from_str(&output.writes[0]).unwrap();
        let content = written["message"]["content"].as_array().unwrap();
        assert_eq!(
            content[0],
            json!({"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "YWJj"}})
        );
        let text = content[1]["text"].as_str().unwrap();
        assert!(text.contains("a.txt (text/plain, 3 bytes): /data/attachments/a.txt"), "{text}");
        assert!(!text.contains("shot.png"), "{text}");
        assert!(text.ends_with("look"), "{text}");
    }
}
