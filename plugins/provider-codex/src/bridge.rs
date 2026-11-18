//! The Codex dialect: `codex app-server` JSON-RPC lines in, deltas and lines out.
//!
//! Verified against codex-cli 0.166.0; the recordings in `fixtures/` are what that
//! version actually sent, and the shapes match its own `generate-ts` output. What the
//! bridge relies on:
//!
//! * `initialize`, the `initialized` notification, then `thread/start` or
//!   `thread/resume`, then one `turn/start` per turn;
//! * everything a turn does arrives as notifications that name its `turnId`, ending
//!   with `turn/completed`;
//! * items have their own ids and lifecycle (`item/started`, deltas,
//!   `item/completed` with the full item);
//! * approvals are requests (`item/commandExecution/requestApproval`,
//!   `item/fileChange/requestApproval`) answered with a `decision`;
//! * the account's usage limits answer `account/rateLimits/read` and come again in
//!   `account/rateLimits/updated` after each turn (see [`crate::limits`]).

use std::collections::{HashMap, HashSet, VecDeque};

use qcc_plugin_sdk::json::str_field;
use qcc_plugin_sdk::jsonrpc::{Incoming, Peer, RpcError};
pub use qcc_plugin_sdk::stdio::Output;
use qcc_plugin_sdk::stdio::{self, LineBridge};
use qcc_plugin_sdk::types::*;
use qcc_plugin_sdk::{Message, boundary, option};
use serde_json::{Value, json};

/// What a pending request of ours is for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Call {
    Initialize,
    /// `thread/start` or `thread/resume`.
    Thread,
    TurnStart,
    /// `thread/compact/start`: the turn it runs is reported like any other.
    Compact,
    Steer,
    Interrupt,
}

/// What waits for the agent to take it: a prompt, or a request to compact.
/// The `input` of a turn: the images, which Codex reads from their files, then the
/// text.
fn input(message: &Message) -> Value {
    let mut input: Vec<Value> = message
        .images
        .iter()
        .map(|image| json!({"type": "localImage", "path": image.path}))
        .collect();
    input.push(json!({"type": "text", "text": message.text, "text_elements": []}));
    Value::Array(input)
}

enum Waiting {
    /// The `input` of a `turn/start`.
    Prompt(Value),
    Compact,
}

struct Approval {
    /// The agent's request id, echoed exactly.
    id: Value,
}

/// What a session is started with.
#[derive(Debug, Default)]
pub struct Config {
    pub cwd: String,
    pub model: Option<String>,
    /// The thread to resume, if any.
    pub resume: Option<String>,
    pub mode: Option<PermissionMode>,
    /// Developer instructions for the thread: what qcc tells the agent at the start
    /// of a session. Sent with `thread/start` and `thread/resume`.
    pub instructions: Option<String>,
    /// How hard the model reasons. Sent as a config override with `thread/start` and
    /// `thread/resume`, so it holds for every turn of the session.
    pub effort: Option<String>,
    /// More directories the sandbox lets the agent write to: the other folders of
    /// the thread's project. Sent as a config override.
    pub writable_roots: Vec<String>,
    /// MCP servers to add to those of the user's Codex configuration. Sent as a
    /// config override.
    pub mcp_servers: Vec<McpServer>,
}

pub struct Bridge {
    peer: Peer<Call>,
    cwd: String,
    model: Option<String>,
    mode: Option<PermissionMode>,
    /// The thread being resumed, until the agent accepted or refused it.
    resume: Option<String>,
    /// The agent's thread, once it is ready for prompts.
    thread: Option<String>,
    /// Prompts not sent yet: the agent is not ready or a turn is running.
    waiting: VecDeque<Waiting>,
    /// The running turn is a compaction we asked for, not one Codex chose to do.
    compacting: bool,
    /// A turn exists on the agent's side, from `turn/start` to `turn/completed`.
    in_flight: bool,
    turn_open: bool,
    /// Codex's id of the running turn, once it told us.
    turn_id: Option<String>,
    /// An interrupt was asked for before the turn had an id to interrupt.
    interrupt_pending: bool,
    /// Messages for the running turn, held until it has an id to steer.
    steers_pending: Vec<Value>,
    /// Turns settled by an interrupt: what the agent still says about them is ignored.
    interrupted: HashSet<String>,
    approvals: HashMap<String, Approval>,
    instructions: Option<String>,
    effort: Option<String>,
    writable_roots: Vec<String>,
    mcp_servers: Vec<McpServer>,
}

/// How qcc introduces itself in `initialize`.
pub fn client_info() -> Value {
    json!({"name": "qcc", "title": "qcc", "version": env!("CARGO_PKG_VERSION")})
}

fn join(values: &Value) -> String {
    values.as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")
}

/// The item a thread item is shown as, if it is one of the non-text kinds.
fn tool_shape(item: &Value) -> Option<ItemShape> {
    Some(match str_field(item, "type") {
        "commandExecution" => ItemShape::Command(CommandItem {
            command: str_field(item, "command").to_owned(),
            cwd: item.get("cwd").and_then(Value::as_str).map(str::to_owned),
            output: item.get("aggregatedOutput").and_then(Value::as_str).map(str::to_owned),
            exit_code: item
                .get("exitCode")
                .and_then(Value::as_i64)
                .and_then(|c| i32::try_from(c).ok()),
        }),
        "fileChange" => {
            let changes: Vec<&Value> = item["changes"].as_array().into_iter().flatten().collect();
            let paths: Vec<&str> = changes.iter().map(|change| str_field(change, "path")).collect();
            let diffs: Vec<&str> = changes
                .iter()
                .map(|change| str_field(change, "diff"))
                .filter(|diff| !diff.is_empty())
                .collect();
            ItemShape::FileChange(FileChangeItem {
                path: paths.join(", "),
                diff: (!diffs.is_empty()).then(|| diffs.join("\n")),
            })
        }
        "mcpToolCall" => ItemShape::Tool(ToolItem {
            name: format!("{}.{}", str_field(item, "server"), str_field(item, "tool")),
            input: item.get("arguments").unwrap_or(&Value::Null).to_string(),
            output: ["result", "error"]
                .iter()
                .find_map(|field| item.get(*field).filter(|value| !value.is_null()))
                .map(Value::to_string),
            mcp_server: Some(str_field(item, "server").to_owned()).filter(|s| !s.is_empty()),
            skill: None,
        }),
        "dynamicToolCall" => ItemShape::Tool(ToolItem {
            name: str_field(item, "tool").to_owned(),
            input: item.get("arguments").unwrap_or(&Value::Null).to_string(),
            output: None,
            mcp_server: None,
            skill: None,
        }),
        "webSearch" => ItemShape::Search(str_field(item, "query").to_owned()),
        _ => return None,
    })
}

/// Channel and final text of a text-kind item.
fn text_item(item: &Value) -> Option<(TextChannel, String)> {
    Some(match str_field(item, "type") {
        "agentMessage" => (TextChannel::AgentMessage, str_field(item, "text").to_owned()),
        "plan" => (TextChannel::Plan, str_field(item, "text").to_owned()),
        "reasoning" => {
            let summary = join(&item["summary"]);
            let text = if summary.is_empty() { join(&item["content"]) } else { summary };
            (TextChannel::Reasoning, text)
        }
        _ => return None,
    })
}

impl Bridge {
    /// A bridge for a new agent process, and the handshake to send it.
    pub fn new(config: Config) -> (Self, Output) {
        let Config { cwd, model, resume, mode, instructions, effort, writable_roots, mcp_servers } =
            config;
        let mut bridge = Self {
            peer: Peer::default(),
            cwd,
            model,
            mode,
            resume,
            thread: None,
            waiting: VecDeque::new(),
            compacting: false,
            in_flight: false,
            turn_open: false,
            turn_id: None,
            interrupt_pending: false,
            steers_pending: Vec::new(),
            interrupted: HashSet::new(),
            approvals: HashMap::new(),
            instructions,
            effort,
            writable_roots,
            mcp_servers,
        };
        let initialize = bridge.peer.request(
            "initialize",
            json!({"clientInfo": client_info()}),
            Call::Initialize,
        );
        (bridge, Output { writes: vec![initialize], ..Output::default() })
    }

    /// The user sent a message. It goes out as soon as the agent can take it.
    pub fn prompt(&mut self, text: &str) -> Output {
        self.prompt_message(&Message { text: text.to_owned(), images: Vec::new() })
    }

    /// [`Bridge::prompt`] with images to look at.
    pub fn prompt_message(&mut self, message: &Message) -> Output {
        self.waiting.push_back(Waiting::Prompt(input(message)));
        let mut output = Output::default();
        self.send_next_prompt(&mut output);
        output
    }

    /// Compact the thread's context, as soon as the agent can take it. Codex runs
    /// it as a turn with a `contextCompaction` item.
    pub fn compact(&mut self) -> Output {
        self.waiting.push_back(Waiting::Compact);
        let mut output = Output::default();
        self.send_next_prompt(&mut output);
        output
    }

    fn send_next_prompt(&mut self, output: &mut Output) {
        if self.thread.is_none() || self.in_flight {
            return;
        }
        let (Some(next), Some(thread)) = (self.waiting.pop_front(), &self.thread) else {
            return;
        };
        let request = match next {
            Waiting::Prompt(input) => self.peer.request(
                "turn/start",
                json!({"threadId": thread, "input": input}),
                Call::TurnStart,
            ),
            Waiting::Compact => {
                self.compacting = true;
                self.peer.request(
                    "thread/compact/start",
                    json!({"threadId": thread}),
                    Call::Compact,
                )
            }
        };
        output.writes.push(request);
        self.in_flight = true;
        self.turn_open = true;
        output.deltas.push(Delta::TurnOpen);
    }

    /// The user sent a message for the running turn.
    pub fn steer(&mut self, text: &str) -> Output {
        self.steer_message(&Message { text: text.to_owned(), images: Vec::new() })
    }

    /// [`Bridge::steer`] with images to look at.
    pub fn steer_message(&mut self, message: &Message) -> Output {
        if !self.turn_open {
            // The turn ended while the message was on its way: it starts the next one.
            return self.prompt_message(message);
        }
        let mut output = Output::default();
        self.steers_pending.push(input(message));
        self.send_steers(&mut output);
        output
    }

    /// `turn/steer` names the turn it expects to be running, so it can only be sent
    /// once the agent has told us the turn's id.
    fn send_steers(&mut self, output: &mut Output) {
        let (Some(turn), Some(thread)) = (&self.turn_id, &self.thread) else {
            return;
        };
        for input in std::mem::take(&mut self.steers_pending) {
            output.writes.push(self.peer.request(
                "turn/steer",
                json!({"threadId": thread, "expectedTurnId": turn, "input": input}),
                Call::Steer,
            ));
        }
    }

    /// Interrupt the running turn. The turn is settled right here; what the agent
    /// still says about it, up to its `turn/completed`, is ignored.
    pub fn interrupt(&mut self) -> Output {
        let mut output = Output::default();
        if !self.turn_open {
            return output;
        }
        for (_, approval) in self.approvals.drain() {
            output.writes.push(self.peer.respond(&approval.id, json!({"decision": "cancel"})));
        }
        match self.turn_id.take() {
            Some(turn) => self.send_interrupt(turn, &mut output),
            // `turn/start` has not been answered yet; interrupt as soon as it is.
            None => self.interrupt_pending = true,
        }
        self.settle(TurnStatus::Interrupted, None, &mut output.deltas);
        output
    }

    fn send_interrupt(&mut self, turn: String, output: &mut Output) {
        if let Some(thread) = &self.thread {
            output.writes.push(self.peer.request(
                "turn/interrupt",
                json!({"threadId": thread, "turnId": turn}),
                Call::Interrupt,
            ));
        }
        self.interrupted.insert(turn);
    }

    /// The user answered an approval request.
    pub fn resolve(
        &mut self,
        request_id: &str,
        resolution: &InteractionResolution,
    ) -> Result<Output, String> {
        let approval = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| format!("no approval request {request_id:?} is pending"))?;
        let decision = match resolution {
            InteractionResolution::Selected(option)
                if option == "accept" || option == "acceptForSession" =>
            {
                option.as_str()
            }
            // `cancel` would also stop the turn; a refusal lets the agent go on.
            InteractionResolution::Selected(_) | InteractionResolution::Text(_) => "decline",
            InteractionResolution::Cancelled => "cancel",
        };
        let response = self.peer.respond(&approval.id, json!({"decision": decision}));
        Ok(Output { writes: vec![response], ..Output::default() })
    }

    /// The session was stopped on request.
    pub fn stopped(&mut self) -> Vec<Delta> {
        stdio::stopped(std::mem::take(&mut self.turn_open))
    }

    fn response(&mut self, call: Call, result: Result<Value, RpcError>, output: &mut Output) {
        match (call, result) {
            (Call::Initialize, Ok(_)) => {
                output.writes.push(self.peer.notify("initialized", Value::Null));
                let request = self.thread_request();
                output.writes.push(request);
                // The account's limits, before the first turn brings them.
                output.writes.push(
                    json!({"jsonrpc": "2.0", "id": crate::limits::READ_ID,
                           "method": crate::limits::READ, "params": {}})
                    .to_string(),
                );
            }
            (Call::Thread, Ok(result)) => {
                let thread = str_field(&result["thread"], "id").to_owned();
                if self.resume.take().as_deref() != Some(thread.as_str()) {
                    output.deltas.push(Delta::SessionIdentity(thread.clone()));
                }
                self.thread = Some(thread);
                self.send_next_prompt(output);
            }
            (Call::Thread, Err(error)) if self.resume.is_some() => {
                // The old thread is gone on the agent's side. A fresh one loses the
                // context, which the user has to know.
                self.resume = None;
                output.deltas.push(Delta::ProviderWarning(format!(
                    "could not resume the previous thread ({}); starting a new one",
                    error.message
                )));
                let request = self.thread_request();
                output.writes.push(request);
            }
            (Call::Initialize | Call::Thread, Err(error)) => {
                let reason = format!("codex refused to start a thread: {}", error.message);
                output.deltas.push(Delta::ProviderError(reason.clone()));
                output.deltas.push(Delta::SessionEnded(Some(reason)));
                output.fatal = true;
            }
            (Call::TurnStart, Ok(result)) => {
                let turn = str_field(&result["turn"], "id").to_owned();
                if std::mem::take(&mut self.interrupt_pending) {
                    self.send_interrupt(turn, output);
                } else if self.turn_open {
                    self.turn_id = Some(turn);
                    self.send_steers(output);
                }
            }
            (Call::Steer, Ok(_)) => {}
            (Call::Steer, Err(error)) => {
                // Most likely the turn ended first. The user has to know the message
                // did not reach the agent.
                output.deltas.push(Delta::ProviderError(format!(
                    "the message could not be added to the running turn: {}",
                    error.message
                )));
            }
            // The turn it starts is announced with `turn/started`.
            (Call::Compact, Ok(_)) => {}
            (Call::TurnStart | Call::Compact, Err(error)) => {
                // The agent never took the prompt: no turn exists on its side.
                self.in_flight = false;
                self.interrupt_pending = false;
                self.settle(TurnStatus::Failed, Some(error.message), &mut output.deltas);
                self.send_next_prompt(output);
            }
            // The turn's own `turn/completed` is what counts.
            (Call::Interrupt, _) => {}
        }
    }

    fn thread_request(&mut self) -> String {
        let mut params = json!({"cwd": self.cwd});
        if let Some(instructions) = &self.instructions {
            params["developerInstructions"] = json!(instructions);
        }
        // Without a mode nothing is sent and the user's own Codex configuration
        // decides, as it would in the Codex CLI. A mode sets all three knobs: the
        // sandbox, when to ask, and who is asked.
        let policy = match self.mode {
            None => None,
            // Ask about everything that is not known to be harmless.
            Some(PermissionMode::Ask) => Some(("untrusted", "workspace-write", Some("user"))),
            // Work freely inside the working directory; ask the user to leave it.
            Some(PermissionMode::AcceptEdits) => {
                Some(("on-request", "workspace-write", Some("user")))
            }
            // The same sandbox, but Codex's automatic reviewer answers.
            Some(PermissionMode::Auto) => {
                Some(("on-request", "workspace-write", Some("auto_review")))
            }
            Some(PermissionMode::Full) => Some(("never", "danger-full-access", None)),
        };
        if let Some((approval, sandbox, reviewer)) = policy {
            params["approvalPolicy"] = json!(approval);
            params["sandbox"] = json!(sandbox);
            if let Some(reviewer) = reviewer {
                params["approvalsReviewer"] = json!(reviewer);
            }
        }
        if let Some(model) = &self.model {
            params["model"] = json!(model);
        }
        let mut config = serde_json::Map::new();
        if let Some(effort) = &self.effort {
            config.insert("model_reasoning_effort".into(), json!(effort));
        }
        if !self.writable_roots.is_empty() {
            config.insert(
                "sandbox_workspace_write".into(),
                json!({"writable_roots": self.writable_roots}),
            );
        }
        if !self.mcp_servers.is_empty() {
            config.insert("mcp_servers".into(), mcp_servers(&self.mcp_servers));
        }
        if !config.is_empty() {
            params["config"] = Value::Object(config);
        }
        match &self.resume {
            Some(thread) => {
                params["threadId"] = json!(thread);
                self.peer.request("thread/resume", params, Call::Thread)
            }
            None => self.peer.request("thread/start", params, Call::Thread),
        }
    }

    fn settle(&mut self, status: TurnStatus, error: Option<String>, deltas: &mut Vec<Delta>) {
        self.compacting = false;
        self.approvals.clear();
        self.steers_pending.clear();
        self.turn_id = None;
        if std::mem::take(&mut self.turn_open) {
            deltas.push(boundary(status, error));
        }
    }

    fn notification(&mut self, method: &str, params: &Value, output: &mut Output) {
        // The account's, whatever the turn is doing.
        if method == "account/rateLimits/updated" {
            output.deltas.extend(crate::limits::from_update(params).map(Delta::RateLimits));
            return;
        }
        // MCP servers start with the thread, before any turn, and report one by one.
        if method == "mcpServer/startupStatus/updated" {
            let state = match str_field(params, "status") {
                "starting" => McpState::Starting,
                "ready" => McpState::Ready,
                "failed" | "cancelled" => McpState::Failed,
                _ => return,
            };
            let error = match params.get("error").and_then(Value::as_str) {
                Some(error) => Some(error.to_owned()),
                None if str_field(params, "status") == "cancelled" => {
                    Some("its startup was cancelled".to_owned())
                }
                None => None,
            };
            let name = str_field(params, "name").to_owned();
            output.deltas.push(Delta::McpStatus(McpStatus { name, state, error }));
            return;
        }
        let turn = match params.get("turnId").and_then(Value::as_str) {
            Some(turn) => turn,
            None => str_field(&params["turn"], "id"),
        };
        if self.interrupted.contains(turn) {
            if method == "turn/completed" {
                self.interrupted.remove(turn);
                self.in_flight = false;
                self.send_next_prompt(output);
            }
            return;
        }
        // Between turns only errors matter: an MCP server or the login failing is
        // still the user's business.
        if !self.turn_open && method != "error" {
            return;
        }
        let deltas = &mut output.deltas;
        match method {
            "turn/started" => {
                self.turn_id = Some(turn.to_owned());
                self.send_steers(output);
            }
            "item/started" => {
                let item = &params["item"];
                if let Some(shape) = tool_shape(item) {
                    let key = ItemKey::Provider(str_field(item, "id").to_owned());
                    deltas.push(Delta::ItemOpen(ItemOpen { key, shape }));
                }
            }
            "item/agentMessage/delta" => text_delta(params, TextChannel::AgentMessage, deltas),
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                text_delta(params, TextChannel::Reasoning, deltas);
            }
            "item/plan/delta" => text_delta(params, TextChannel::Plan, deltas),
            "item/completed" => {
                let item = &params["item"];
                let key = ItemKey::Provider(str_field(item, "id").to_owned());
                if str_field(item, "type") == "contextCompaction" {
                    deltas.push(Delta::ContextCompacted(Compaction {
                        automatic: !self.compacting,
                        tokens_before: None,
                        tokens_after: None,
                    }));
                } else if let Some((channel, text)) = text_item(item) {
                    // Reasoning is often withheld: the item comes, but empty.
                    if !text.is_empty() {
                        deltas.push(Delta::ItemTextClose(TextClose {
                            key,
                            channel,
                            text: Some(text),
                        }));
                    }
                } else if let Some(shape) = tool_shape(item) {
                    let status = match str_field(item, "status") {
                        "failed" => ItemStatus::Failed,
                        "declined" => ItemStatus::Declined,
                        _ => ItemStatus::Completed,
                    };
                    deltas.push(Delta::ItemClose(ItemClose { key, shape, status }));
                }
            }
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"];
                let breakdown = |part: &Value| TokenUsage {
                    input_tokens: part.get("inputTokens").and_then(Value::as_u64).unwrap_or(0),
                    output_tokens: part.get("outputTokens").and_then(Value::as_u64).unwrap_or(0),
                    cached_input_tokens: part.get("cachedInputTokens").and_then(Value::as_u64),
                };
                // Codex keeps exact cumulative totals; both go through as reported.
                let last = breakdown(&usage["last"]);
                // Its cached input is part of its input, and what the last request
                // read and wrote is what the next one carries: Codex itself counts
                // the context as the last request's total.
                let context_used = usage["last"]
                    .get("totalTokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(last.input_tokens + last.output_tokens);
                deltas.push(Delta::Usage(UsageSnapshot {
                    total: breakdown(&usage["total"]),
                    last,
                    model_context_window: usage.get("modelContextWindow").and_then(Value::as_u64),
                    context_used: Some(context_used),
                }));
            }
            "turn/completed" => {
                let turn = &params["turn"];
                let (status, error) = match str_field(turn, "status") {
                    "interrupted" => (TurnStatus::Interrupted, None),
                    "failed" => {
                        let message = str_field(&turn["error"], "message");
                        (TurnStatus::Failed, Some(message.to_owned()).filter(|m| !m.is_empty()))
                    }
                    _ => (TurnStatus::Completed, None),
                };
                self.in_flight = false;
                self.settle(status, error, deltas);
                self.send_next_prompt(output);
            }
            "error" => {
                let message = str_field(&params["error"], "message").to_owned();
                let retrying = params.get("willRetry").and_then(Value::as_bool).unwrap_or(false);
                deltas.push(if retrying {
                    Delta::ProviderWarning(message)
                } else {
                    Delta::ProviderError(message)
                });
            }
            // The app-server narrates a great deal the timeline has no use for:
            // MCP startup, hooks, account and rate-limit state, thread status.
            _ => {}
        }
    }

    fn request(&mut self, id: Value, method: &str, params: &Value, output: &mut Output) {
        let (default_title, detail) = match method {
            "item/commandExecution/requestApproval" => {
                ("Run this command?", params.get("command").and_then(Value::as_str))
            }
            "item/fileChange/requestApproval" => ("Apply these changes?", None),
            _ => {
                output.writes.push(self.peer.refuse(&id, method));
                return;
            }
        };
        if !self.turn_open || self.interrupted.contains(str_field(params, "turnId")) {
            output.writes.push(self.peer.respond(&id, json!({"decision": "cancel"})));
            return;
        }
        let title = match str_field(params, "reason") {
            "" => default_title,
            reason => reason,
        };
        let request_id = id.to_string();
        output.deltas.push(Delta::InteractionRequest(InteractionAsk {
            request_id: request_id.clone(),
            item: Some(ItemKey::Provider(str_field(params, "itemId").to_owned())),
            request: InteractionRequest::Approval(ApprovalRequest {
                title: title.to_owned(),
                detail: detail.map(str::to_owned),
                options: vec![
                    option("accept", "Allow", OptionKind::Allow),
                    option("acceptForSession", "Allow for this session", OptionKind::Allow),
                    option("decline", "Deny", OptionKind::Deny),
                ],
            }),
        }));
        self.approvals.insert(request_id, Approval { id });
    }
}

fn text_delta(params: &Value, channel: TextChannel, deltas: &mut Vec<Delta>) {
    let text = str_field(params, "delta");
    if !text.is_empty() {
        deltas.push(Delta::ItemTextDelta(TextDelta {
            key: ItemKey::Provider(str_field(params, "itemId").to_owned()),
            channel,
            text: text.to_owned(),
        }));
    }
}

impl LineBridge for Bridge {
    /// One line of the agent's stdout.
    fn on_line(&mut self, line: &str) -> Output {
        let mut output = Output::default();
        // The answer to our own read of the limits, which the peer does not number.
        if line.contains(crate::limits::READ_ID)
            && let Ok(message) = serde_json::from_str::<Value>(line)
            && message["id"] == crate::limits::READ_ID
        {
            // An account without such limits (an API key) answers with an error.
            if let Some(result) = message.get("result") {
                output.deltas.push(Delta::RateLimits(crate::limits::from_read(result)));
            }
            return output;
        }
        match self.peer.classify(line) {
            Incoming::Response { tag, result } => self.response(tag, result, &mut output),
            Incoming::Notification { method, params } => {
                self.notification(&method, &params, &mut output);
            }
            Incoming::Request { id, method, params } => {
                self.request(id, &method, &params, &mut output)
            }
            Incoming::Unknown(_) => {}
        }
        output
    }

    /// Input still waiting for the agent is lost with it, which is an error too.
    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta> {
        let turn_open = std::mem::take(&mut self.turn_open);
        let error_if_idle = code != Some(0) || !self.waiting.is_empty();
        stdio::exited("codex", turn_open, error_if_idle, code, stderr)
    }
}

/// The `mcp_servers` table of Codex's configuration for qcc's servers. Codex
/// merges it into the user's own.
fn mcp_servers(servers: &[McpServer]) -> Value {
    let pairs = |pairs: &[(String, String)]| -> serde_json::Map<String, Value> {
        pairs.iter().map(|(name, value)| (name.clone(), json!(value))).collect()
    };
    let servers: serde_json::Map<String, Value> = servers
        .iter()
        .map(|server| {
            let config = match &server.transport {
                McpTransport::Stdio(stdio) => {
                    json!({"command": stdio.command, "args": stdio.args, "env": pairs(&stdio.env)})
                }
                McpTransport::Http(http) => {
                    json!({"url": http.url, "http_headers": pairs(&http.headers)})
                }
            };
            (server.name.clone(), config)
        })
        .collect();
    Value::Object(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config { cwd: "/work".into(), ..Config::default() }
    }

    /// The methods written, but for the read of the account's limits that follows
    /// every handshake (`handshake_then_prompt` checks it).
    fn methods(output: &Output) -> Vec<String> {
        output
            .writes
            .iter()
            .map(|line| {
                let value: Value = serde_json::from_str(line).unwrap();
                value["method"].as_str().unwrap_or("response").to_owned()
            })
            .filter(|method| method != crate::limits::READ)
            .collect()
    }

    fn notify(method: &str, params: Value) -> String {
        json!({"method": method, "params": params}).to_string()
    }

    /// A bridge with thread `th` ready and a turn started (`turn/start` is request 3).
    fn started() -> Bridge {
        let (mut bridge, _) = Bridge::new(config());
        bridge.on_line(r#"{"id":1,"result":{}}"#);
        bridge.on_line(r#"{"id":2,"result":{"thread":{"id":"th"}}}"#);
        assert_eq!(bridge.prompt("go").deltas, [Delta::TurnOpen]);
        bridge
    }

    #[test]
    fn compacting_waits_for_the_thread_and_runs_as_a_turn() {
        let (mut bridge, _) = Bridge::new(Config { resume: Some("old".into()), ..config() });
        assert_eq!(bridge.compact(), Output::default(), "held until the thread is resumed");
        bridge.on_line(r#"{"id":1,"result":{}}"#);
        let ready = bridge.on_line(r#"{"id":2,"result":{"thread":{"id":"old"}}}"#);
        assert_eq!(methods(&ready), ["thread/compact/start"]);
        assert!(ready.deltas.contains(&Delta::TurnOpen));
        bridge.on_line(r#"{"id":3,"result":{}}"#);
        bridge.on_line(&notify("turn/started", json!({"turn": {"id": "t"}})));
        let item = json!({"type": "contextCompaction", "id": "c"});
        let compacted =
            bridge.on_line(&notify("item/completed", json!({"turnId": "t", "item": item})));
        assert_eq!(
            compacted.deltas,
            [Delta::ContextCompacted(Compaction {
                automatic: false,
                tokens_before: None,
                tokens_after: None
            })]
        );
        let done = bridge.on_line(&notify(
            "turn/completed",
            json!({"turn": {"id": "t", "status": "completed"}}),
        ));
        assert!(matches!(done.deltas[..], [Delta::TurnBoundary(_)]), "{:?}", done.deltas);

        // A compaction Codex chose to do in a turn of the user's is its own.
        let mut bridge = started();
        bridge.on_line(r#"{"id":3,"result":{"turn":{"id":"u"}}}"#);
        let item = json!({"type": "contextCompaction", "id": "c2"});
        let auto = bridge.on_line(&notify("item/completed", json!({"turnId": "u", "item": item})));
        let [Delta::ContextCompacted(compaction)] = &auto.deltas[..] else {
            panic!("expected a compaction: {:?}", auto.deltas);
        };
        assert!(compaction.automatic);
    }

    #[test]
    fn handshake_then_prompt() {
        let (mut bridge, handshake) =
            Bridge::new(Config { model: Some("gpt-x".into()), ..config() });
        assert_eq!(methods(&handshake), ["initialize"]);
        assert_eq!(bridge.prompt("hello"), Output::default(), "held until the thread exists");

        let initialized = bridge.on_line(r#"{"id":1,"result":{"userAgent":"x"}}"#);
        assert_eq!(methods(&initialized), ["initialized", "thread/start"]);
        assert!(initialized.writes[2].contains("\"id\":\"qcc-rate-limits\""));
        assert!(initialized.writes[1].contains("\"model\":\"gpt-x\""));
        assert!(!initialized.writes[1].contains("writable_roots"));
        assert!(!initialized.writes[1].contains("model_reasoning_effort"));

        let ready = bridge.on_line(r#"{"id":2,"result":{"thread":{"id":"th"}}}"#);
        assert_eq!(methods(&ready), ["turn/start"]);
        assert_eq!(ready.deltas, [Delta::SessionIdentity("th".into()), Delta::TurnOpen]);
        assert!(ready.writes[0].contains("\"threadId\":\"th\""));
    }

    #[test]
    fn the_effort_holds_for_the_session() {
        let (mut bridge, _) = Bridge::new(Config {
            resume: Some("old".into()),
            effort: Some("high".into()),
            ..config()
        });
        let resume = bridge.on_line(r#"{"id":1,"result":{}}"#);
        assert_eq!(methods(&resume), ["initialized", "thread/resume"]);
        assert!(resume.writes[1].contains("\"model_reasoning_effort\":\"high\""));
    }

    #[test]
    fn the_project_folders_are_writable_roots() {
        let (mut bridge, _) =
            Bridge::new(Config { writable_roots: vec!["/api".into(), "/web".into()], ..config() });
        let initialized = bridge.on_line(r#"{"id":1,"result":{}}"#);
        let start: serde_json::Value = serde_json::from_str(&initialized.writes[1]).unwrap();
        assert_eq!(
            start["params"]["config"],
            json!({"sandbox_workspace_write": {"writable_roots": ["/api", "/web"]}})
        );
    }

    #[test]
    fn mcp_startup_is_reported_between_turns() {
        let (mut bridge, _) = Bridge::new(config());
        bridge.on_line(r#"{"id":1,"result":{}}"#);
        let status = |name: &str, status: &str, error: Value| {
            notify(
                "mcpServer/startupStatus/updated",
                json!({"threadId": "th", "name": name, "status": status, "error": error}),
            )
        };
        assert_eq!(
            bridge.on_line(&status("docs", "starting", Value::Null)).deltas,
            [Delta::McpStatus(McpStatus {
                name: "docs".into(),
                state: McpState::Starting,
                error: None
            })]
        );
        assert_eq!(
            bridge.on_line(&status("docs", "failed", json!("MCP startup failed"))).deltas,
            [Delta::McpStatus(McpStatus {
                name: "docs".into(),
                state: McpState::Failed,
                error: Some("MCP startup failed".into())
            })]
        );
        assert!(bridge.on_line(&status("docs", "brand-new", Value::Null)).deltas.is_empty());
    }

    #[test]
    fn mcp_servers_are_a_config_override() {
        let (mut bridge, _) = Bridge::new(Config {
            resume: Some("old".into()),
            writable_roots: vec!["/api".into()],
            mcp_servers: vec![
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
            ],
            ..config()
        });
        let initialized = bridge.on_line(r#"{"id":1,"result":{}}"#);
        assert_eq!(methods(&initialized), ["initialized", "thread/resume"]);
        let resume: Value = serde_json::from_str(&initialized.writes[1]).unwrap();
        assert_eq!(
            resume["params"]["config"],
            json!({
                "sandbox_workspace_write": {"writable_roots": ["/api"]},
                "mcp_servers": {
                    "docs": {"command": "npx", "args": ["docs-mcp"], "env": {"TOKEN": "t"}},
                    "web": {"url": "https://example.com/mcp",
                            "http_headers": {"Authorization": "Bearer x"}},
                },
            })
        );
    }

    #[test]
    fn resume_falls_back_to_a_new_thread() {
        let (mut bridge, _) = Bridge::new(Config { resume: Some("old".into()), ..config() });
        let resume = bridge.on_line(r#"{"id":1,"result":{}}"#);
        assert_eq!(methods(&resume), ["initialized", "thread/resume"]);
        assert!(resume.writes[1].contains("\"threadId\":\"old\""));

        let gone =
            bridge.on_line(r#"{"id":2,"error":{"code":-32600,"message":"no rollout found"}}"#);
        assert_eq!(methods(&gone), ["thread/start"]);
        assert!(
            matches!(&gone.deltas[..], [Delta::ProviderWarning(w)] if w.contains("no rollout found"))
        );
        let fresh = bridge.on_line(r#"{"id":3,"result":{"thread":{"id":"new"}}}"#);
        assert_eq!(fresh.deltas, [Delta::SessionIdentity("new".into())]);

        // A thread that resumes under its own id announces nothing.
        let (mut bridge, _) = Bridge::new(Config { resume: Some("old".into()), ..config() });
        bridge.on_line(r#"{"id":1,"result":{}}"#);
        assert!(bridge.on_line(r#"{"id":2,"result":{"thread":{"id":"old"}}}"#).deltas.is_empty());
    }

    #[test]
    fn an_interrupt_before_the_turn_has_an_id_is_sent_when_it_gets_one() {
        let mut bridge = started();
        let early = bridge.interrupt();
        assert_eq!(methods(&early), Vec::<String>::new());
        assert!(
            matches!(&early.deltas[..], [Delta::TurnBoundary(b)] if b.status == TurnStatus::Interrupted)
        );

        let late = bridge.on_line(r#"{"id":3,"result":{"turn":{"id":"t1"}}}"#);
        assert_eq!(methods(&late), ["turn/interrupt"]);
        assert!(late.writes[0].contains("\"turnId\":\"t1\""));

        // The next prompt waits until the agent has finished with the old turn.
        assert_eq!(bridge.prompt("next"), Output::default());
        let stale =
            notify("item/agentMessage/delta", json!({"turnId": "t1", "itemId": "m", "delta": "x"}));
        assert_eq!(bridge.on_line(&stale), Output::default());
        let done = notify("turn/completed", json!({"turn": {"id": "t1", "status": "interrupted"}}));
        let next = bridge.on_line(&done);
        assert_eq!(methods(&next), ["turn/start"]);
        assert_eq!(next.deltas, [Delta::TurnOpen]);
    }

    #[test]
    fn the_permission_mode_sets_sandbox_policy_and_reviewer() {
        let thread_start = |mode| {
            let (mut bridge, _) = Bridge::new(Config { mode, ..config() });
            let output = bridge.on_line(r#"{"id":1,"result":{}}"#);
            let request: Value = serde_json::from_str(&output.writes[1]).unwrap();
            request["params"].clone()
        };
        let knobs = |mode| {
            let params = thread_start(Some(mode));
            let text = |name: &str| params[name].as_str().unwrap_or("-").to_owned();
            [text("approvalPolicy"), text("sandbox"), text("approvalsReviewer")].join(" ")
        };
        assert_eq!(knobs(PermissionMode::Ask), "untrusted workspace-write user");
        assert_eq!(knobs(PermissionMode::AcceptEdits), "on-request workspace-write user");
        // `auto` is the provider's own reviewer, not ours.
        assert_eq!(knobs(PermissionMode::Auto), "on-request workspace-write auto_review");
        assert_eq!(knobs(PermissionMode::Full), "never danger-full-access -");
        // No mode: the user's Codex configuration is left alone.
        assert_eq!(thread_start(None), json!({"cwd": "/work"}));
    }

    #[test]
    fn steering_waits_for_the_turn_id_and_names_it() {
        let mut bridge = started();
        assert_eq!(bridge.steer("also this"), Output::default(), "the turn has no id yet");
        let known = bridge.on_line(r#"{"id":3,"result":{"turn":{"id":"t1"}}}"#);
        assert_eq!(methods(&known), ["turn/steer"]);
        assert!(known.writes[0].contains("\"expectedTurnId\":\"t1\""));
        assert!(known.writes[0].contains("also this"));
        assert!(known.deltas.is_empty(), "a steer opens no turn");

        assert_eq!(methods(&bridge.steer("and that")), ["turn/steer"]);
        // The turn was already over on the agent's side.
        let refused =
            bridge.on_line(r#"{"id":5,"error":{"code":-32600,"message":"no active turn"}}"#);
        assert!(
            matches!(&refused.deltas[..], [Delta::ProviderError(e)] if e.contains("no active turn"))
        );

        // With no turn running, a steer is simply the next prompt.
        let done = notify("turn/completed", json!({"turn": {"id": "t1", "status": "completed"}}));
        bridge.on_line(&done);
        let next = bridge.steer("late");
        assert_eq!(methods(&next), ["turn/start"]);
        assert_eq!(next.deltas, [Delta::TurnOpen]);
    }

    #[test]
    fn a_refused_turn_fails_and_the_queue_moves_on() {
        let mut bridge = started();
        bridge.prompt("second");
        let refused =
            bridge.on_line(r#"{"id":3,"error":{"code":-32000,"message":"usage limit reached"}}"#);
        assert_eq!(methods(&refused), ["turn/start"]);
        assert!(matches!(
            &refused.deltas[..],
            [Delta::TurnBoundary(b), Delta::TurnOpen]
                if b.status == TurnStatus::Failed && b.error.as_deref() == Some("usage limit reached")
        ));
    }

    #[test]
    fn items_map_to_shapes() {
        let mut bridge = started();
        let change = json!({"type": "fileChange", "id": "fc", "status": "completed", "changes": [
            {"path": "a.rs", "kind": "update", "diff": "@@ a"},
            {"path": "b.rs", "kind": "add", "diff": "@@ b"},
        ]});
        let closed =
            bridge.on_line(&notify("item/completed", json!({"turnId": "t", "item": change})));
        let [Delta::ItemClose(close)] = &closed.deltas[..] else { panic!("{closed:?}") };
        assert_eq!(
            close.shape,
            ItemShape::FileChange(FileChangeItem {
                path: "a.rs, b.rs".into(),
                diff: Some("@@ a\n@@ b".into())
            })
        );

        let mcp = json!({"type": "mcpToolCall", "id": "m", "server": "docs", "tool": "search",
            "status": "failed", "arguments": {"q": 1}, "result": null, "error": {"message": "boom"}});
        let closed = bridge.on_line(&notify("item/completed", json!({"turnId": "t", "item": mcp})));
        let [Delta::ItemClose(close)] = &closed.deltas[..] else { panic!("{closed:?}") };
        assert_eq!(close.status, ItemStatus::Failed);
        assert!(
            matches!(&close.shape, ItemShape::Tool(t) if t.name == "docs.search" && t.output.is_some())
        );

        // Empty reasoning and the echo of the user's message make no items.
        let reasoning = json!({"type": "reasoning", "id": "r", "summary": [], "content": []});
        let user = json!({"type": "userMessage", "id": "u", "content": []});
        for item in [reasoning, user] {
            let output =
                bridge.on_line(&notify("item/completed", json!({"turnId": "t", "item": item})));
            assert_eq!(output, Output::default());
        }
        let summary =
            json!({"type": "reasoning", "id": "r2", "summary": ["a", "b"], "content": []});
        let output =
            bridge.on_line(&notify("item/completed", json!({"turnId": "t", "item": summary})));
        assert!(
            matches!(&output.deltas[..], [Delta::ItemTextClose(c)] if c.text.as_deref() == Some("a\nb"))
        );
    }

    #[test]
    fn errors_and_unknown_requests() {
        let mut bridge = started();
        let retry = notify(
            "error",
            json!({"turnId": "t", "willRetry": true, "error": {"message": "reconnecting"}}),
        );
        assert_eq!(bridge.on_line(&retry).deltas, [Delta::ProviderWarning("reconnecting".into())]);
        let fatal = notify(
            "error",
            json!({"turnId": "t", "willRetry": false, "error": {"message": "nope"}}),
        );
        assert_eq!(bridge.on_line(&fatal).deltas, [Delta::ProviderError("nope".into())]);
        // Between turns too.
        let (mut idle, _) = Bridge::new(config());
        let mcp = notify("error", json!({"willRetry": false, "error": {"message": "mcp down"}}));
        assert_eq!(idle.on_line(&mcp).deltas, [Delta::ProviderError("mcp down".into())]);

        let refused =
            bridge.on_line(r#"{"id":9,"method":"item/tool/requestUserInput","params":{}}"#);
        assert!(refused.writes[0].contains("-32601"));
        assert!(bridge.resolve("nope", &InteractionResolution::Cancelled).is_err());

        let failed = notify(
            "turn/completed",
            json!({"turn": {"id": "t", "status": "failed", "error": {"message": "model overloaded"}}}),
        );
        assert!(matches!(
            &bridge.on_line(&failed).deltas[..],
            [Delta::TurnBoundary(b)] if b.error.as_deref() == Some("model overloaded")
        ));
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
    fn images_are_read_by_codex_from_their_files() {
        let attachments = [attachment("shot.png", "image/png", Some(b"abc"))];
        let message = qcc_plugin_sdk::message("look", &[], &attachments, true);
        let (mut bridge, _) = Bridge::new(config());
        bridge.on_line(r#"{"id":1,"result":{}}"#);
        bridge.on_line(r#"{"id":2,"result":{"thread":{"id":"th"}}}"#);
        let output = bridge.prompt_message(&message);
        assert_eq!(methods(&output), ["turn/start"]);
        let request: Value = serde_json::from_str(&output.writes[0]).unwrap();
        assert_eq!(
            request["params"]["input"],
            json!([
                {"type": "localImage", "path": "/data/attachments/shot.png"},
                {"type": "text", "text": "look", "text_elements": []},
            ])
        );
    }
}
