//! The ACP dialect: JSON-RPC lines in, deltas and JSON-RPC lines out.
//!
//! Verified against opencode 1.19.4 (`opencode acp`); the recordings in `fixtures/`
//! are what that version actually sent. What the bridge relies on:
//!
//! * `initialize`, then `session/new` (or `session/resume`, or `session/load`, to
//!   continue one), then one `session/prompt` per turn, answered when the turn ends;
//! * everything in between arrives as `session/update` notifications;
//! * the agent asks for permission with a `session/request_permission` request —
//!   whether it asks at all is the agent's own configuration; the bridge answers for
//!   the user where the thread's permission mode says so;
//! * `session/cancel` is a notification, and the cancelled prompt still gets its
//!   response.

use std::collections::{HashMap, HashSet, VecDeque};

use qcc_plugin_sdk::json::str_field;
use qcc_plugin_sdk::jsonrpc::{Incoming, Peer, RpcError};
pub use qcc_plugin_sdk::stdio::Output;
use qcc_plugin_sdk::stdio::{self, LineBridge};
use qcc_plugin_sdk::types::*;
use qcc_plugin_sdk::{add_usage, boundary};
use serde_json::{Value, json};

/// What a pending request of ours is for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Call {
    Initialize,
    NewSession,
    /// `session/resume` or `session/load`.
    Continue,
    SetModel,
    Prompt,
}

#[derive(Clone, Copy, PartialEq)]
enum Stream {
    Message,
    Thought,
}

impl Stream {
    fn key(self) -> ItemKey {
        ItemKey::Anonymous(match self {
            Self::Message => "assistant".into(),
            Self::Thought => "thought".into(),
        })
    }

    fn channel(self) -> TextChannel {
        match self {
            Self::Message => TextChannel::AgentMessage,
            Self::Thought => TextChannel::Reasoning,
        }
    }
}

/// What is known about a tool call so far. ACP sends it piecemeal: a bare
/// `tool_call`, then updates that add the input, the locations and the result.
#[derive(Default)]
struct Tool {
    kind: String,
    title: String,
    status: String,
    input: Value,
    path: Option<String>,
    output: Option<String>,
    opened: bool,
}

struct Approval {
    /// The agent's request id, echoed exactly.
    id: Value,
    tool: String,
    /// Option ids that mean "no".
    rejecting: Vec<String>,
    offered: Vec<String>,
}

/// What a session is started with.
#[derive(Debug, Default)]
pub struct Config {
    /// The agent's name, which names it when its process dies.
    pub agent: &'static str,
    pub cwd: String,
    pub model: Option<String>,
    /// The session to continue, if any.
    pub resume: Option<String>,
    pub mode: Option<PermissionMode>,
    /// What qcc tells the agent at the start of a session (see [`Bridge`]'s
    /// `context`).
    pub context: Option<String>,
    /// MCP servers to add to those of the agent's own configuration.
    pub mcp_servers: Vec<McpServer>,
}

pub struct Bridge {
    agent: &'static str,
    peer: Peer<Call>,
    cwd: String,
    model: Option<String>,
    mode: Option<PermissionMode>,
    /// The session being continued, until the agent accepted or refused it.
    resume: Option<String>,
    /// The agent's session, once it is ready for prompts.
    session: Option<String>,
    /// `session/load` replays the old conversation as updates; they are not news.
    replaying: bool,
    /// Prompts not sent yet: the agent is not ready or another prompt is in flight.
    waiting: VecDeque<String>,
    in_flight: bool,
    turn_open: bool,
    /// The in-flight prompt was cancelled and settled; its response is to be ignored.
    cancelled: bool,
    stream: Option<Stream>,
    tools: HashMap<String, Tool>,
    closed: HashSet<String>,
    approvals: HashMap<String, Approval>,
    /// Tool calls the user refused: they settle as declined, not failed.
    refused: HashSet<String>,
    total: TokenUsage,
    context_window: Option<u64>,
    /// What the session must know before its first prompt: what qcc tells the agent
    /// at the start of a session (its skills, the project's other folders).
    /// ACP has no system prompt, so it goes in front of the first prompt of a fresh
    /// session; a continued session has it in its history already.
    context: Option<String>,
    /// qcc's MCP servers, added to the agent's own with every session request.
    mcp_servers: Vec<McpServer>,
    /// `mcpServers` as the agent takes them, once it said what it supports.
    mcp_param: Value,
}

fn first_str<'a>(value: &'a Value, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|name| value.get(*name).and_then(Value::as_str))
}

impl Tool {
    /// Fold a `tool_call`, a `tool_call_update` or a permission request's `toolCall`.
    fn merge(&mut self, update: &Value) {
        for (field, slot) in
            [("kind", &mut self.kind), ("title", &mut self.title), ("status", &mut self.status)]
        {
            if let Some(value) = update.get(field).and_then(Value::as_str) {
                *slot = value.to_owned();
            }
        }
        if let Some(input) = update.get("rawInput").filter(|input| !is_empty(input)) {
            // Updates repeat the input with different detail; keep every field seen.
            match (&mut self.input, input) {
                (Value::Object(known), Value::Object(new)) => known.extend(new.clone()),
                _ => self.input = input.clone(),
            }
        }
        let location = update["locations"].get(0).and_then(|location| location.get("path"));
        if let Some(path) = location.and_then(Value::as_str) {
            self.path = Some(path.to_owned());
        }
        let text: Vec<&str> = update["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|entry| entry["content"].get("text").and_then(Value::as_str))
            .collect();
        if !text.is_empty() {
            self.output = Some(text.join("\n"));
        }
    }

    /// Enough is known to show the call as more than a bare name.
    fn has_details(&self) -> bool {
        !is_empty(&self.input) || self.path.is_some() || self.status != "pending"
    }

    fn shape(&self) -> ItemShape {
        let path = || {
            self.path
                .clone()
                .or_else(|| {
                    first_str(&self.input, &["filePath", "filepath", "path"]).map(str::to_owned)
                })
                .unwrap_or_else(|| self.title.clone())
        };
        match self.kind.as_str() {
            "edit" | "delete" | "move" => ItemShape::FileChange(FileChangeItem {
                path: path(),
                diff: first_str(&self.input, &["diff"]).map(str::to_owned),
            }),
            "read" => ItemShape::FileRead(path()),
            "execute" => ItemShape::Command(CommandItem {
                command: first_str(&self.input, &["command"]).unwrap_or(&self.title).to_owned(),
                cwd: first_str(&self.input, &["cwd", "workdir"]).map(str::to_owned),
                output: self.output.clone(),
                exit_code: None,
            }),
            "search" => ItemShape::Search(
                first_str(&self.input, &["pattern", "query"]).unwrap_or(&self.title).to_owned(),
            ),
            _ => ItemShape::Tool(ToolItem {
                name: self.title.clone(),
                input: self.input.to_string(),
                output: self.output.clone(),
            }),
        }
    }
}

fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

impl Bridge {
    /// A bridge for a new agent process, and the handshake to send it.
    pub fn new(config: Config) -> (Self, Output) {
        let Config { agent, cwd, model, resume, mode, context, mcp_servers } = config;
        let mut bridge = Self {
            agent,
            peer: Peer::default(),
            cwd,
            model,
            mode,
            resume,
            session: None,
            replaying: false,
            waiting: VecDeque::new(),
            in_flight: false,
            turn_open: false,
            cancelled: false,
            stream: None,
            tools: HashMap::new(),
            closed: HashSet::new(),
            approvals: HashMap::new(),
            refused: HashSet::new(),
            total: TokenUsage { input_tokens: 0, output_tokens: 0, cached_input_tokens: None },
            context_window: None,
            context,
            mcp_servers,
            mcp_param: json!([]),
        };
        let initialize = bridge.peer.request(
            "initialize",
            json!({
                "protocolVersion": 1,
                // The agent works on the thread's directory itself; we lend it nothing.
                "clientCapabilities": {
                    "fs": {"readTextFile": false, "writeTextFile": false},
                    "terminal": false,
                },
                "clientInfo": {"name": "qcc", "version": env!("CARGO_PKG_VERSION")},
            }),
            Call::Initialize,
        );
        (bridge, Output { writes: vec![initialize], ..Output::default() })
    }

    /// The user sent a message. It goes out as soon as the agent can take it.
    pub fn prompt(&mut self, text: &str) -> Output {
        self.waiting.push_back(text.to_owned());
        let mut output = Output::default();
        self.send_next_prompt(&mut output);
        output
    }

    fn send_next_prompt(&mut self, output: &mut Output) {
        if self.session.is_none() || self.in_flight {
            return;
        }
        let (Some(text), Some(session)) = (self.waiting.pop_front(), &self.session) else {
            return;
        };
        let mut prompt = Vec::new();
        if let Some(context) = self.context.take() {
            prompt.push(json!({"type": "text", "text": context}));
        }
        prompt.push(json!({"type": "text", "text": text}));
        output.writes.push(self.peer.request(
            "session/prompt",
            json!({"sessionId": session, "prompt": prompt}),
            Call::Prompt,
        ));
        self.in_flight = true;
        self.turn_open = true;
        output.deltas.push(Delta::TurnOpen);
    }

    /// Interrupt the running turn. The turn is settled right here; what the agent
    /// still says about it, up to the prompt's response, is ignored.
    pub fn interrupt(&mut self) -> Output {
        let mut output = Output::default();
        let (true, Some(session)) = (self.turn_open, &self.session) else {
            return output;
        };
        output.writes.push(self.peer.notify("session/cancel", json!({"sessionId": session})));
        // The protocol wants every pending permission request answered as cancelled.
        for (_, approval) in self.approvals.drain() {
            let cancelled = json!({"outcome": {"outcome": "cancelled"}});
            output.writes.push(self.peer.respond(&approval.id, cancelled));
        }
        self.cancelled = true;
        self.settle(TurnStatus::Interrupted, None, &mut output.deltas);
        output
    }

    /// The user answered a permission request.
    pub fn resolve(
        &mut self,
        request_id: &str,
        resolution: &InteractionResolution,
    ) -> Result<Output, String> {
        let approval = self
            .approvals
            .remove(request_id)
            .ok_or_else(|| format!("no permission request {request_id:?} is pending"))?;
        let outcome = match resolution {
            InteractionResolution::Selected(option) if approval.offered.contains(option) => {
                if approval.rejecting.contains(option) {
                    self.refused.insert(approval.tool.clone());
                }
                json!({"outcome": "selected", "optionId": option})
            }
            _ => {
                self.refused.insert(approval.tool.clone());
                json!({"outcome": "cancelled"})
            }
        };
        let response = self.peer.respond(&approval.id, json!({"outcome": outcome}));
        Ok(Output { writes: vec![response], ..Output::default() })
    }

    /// The session was stopped on request.
    pub fn stopped(&mut self) -> Vec<Delta> {
        stdio::stopped(std::mem::take(&mut self.turn_open))
    }

    fn response(&mut self, call: Call, result: Result<Value, RpcError>, output: &mut Output) {
        match (call, result) {
            (Call::Initialize, Ok(result)) => {
                let capabilities = &result["agentCapabilities"];
                let can_resume = capabilities["sessionCapabilities"].get("resume").is_some();
                let can_load = capabilities["loadSession"].as_bool().unwrap_or(false);
                let http = capabilities["mcpCapabilities"]["http"].as_bool().unwrap_or(false);
                self.mcp_param = self.session_mcp_servers(http, &mut output.deltas);
                let request = match &self.resume {
                    Some(session) if can_resume || can_load => {
                        // `resume` picks the session up silently; `load` replays it.
                        self.replaying = !can_resume;
                        let method = if can_resume { "session/resume" } else { "session/load" };
                        let params = json!({"sessionId": session, "cwd": self.cwd, "mcpServers": self.mcp_param});
                        self.peer.request(method, params, Call::Continue)
                    }
                    Some(_) => {
                        let warning = "the agent cannot continue sessions; starting a new one";
                        output.deltas.push(Delta::ProviderWarning(warning.into()));
                        self.new_session()
                    }
                    None => self.new_session(),
                };
                output.writes.push(request);
            }
            (Call::NewSession, Ok(result)) => {
                let session = str_field(&result, "sessionId").to_owned();
                output.deltas.push(Delta::SessionIdentity(session.clone()));
                self.session_ready(session, output);
            }
            (Call::Continue, Ok(_)) => {
                self.replaying = false;
                self.context = None;
                let session = self.resume.take().unwrap_or_default();
                self.session_ready(session, output);
            }
            (Call::Continue, Err(error)) => {
                // The old session is gone on the agent's side. A fresh one loses the
                // context, which the user has to know.
                self.replaying = false;
                self.resume = None;
                output.deltas.push(Delta::ProviderWarning(format!(
                    "could not continue the previous session ({}); starting a new one",
                    error.message
                )));
                output.writes.push(self.new_session());
            }
            (Call::Initialize | Call::NewSession, Err(error)) => {
                let reason = format!("the agent refused to start a session: {}", error.message);
                output.deltas.push(Delta::ProviderError(reason.clone()));
                output.deltas.push(Delta::SessionEnded(Some(reason)));
                output.fatal = true;
            }
            (Call::SetModel, Err(error)) => {
                output.deltas.push(Delta::ProviderWarning(format!(
                    "could not select the model: {}",
                    error.message
                )));
            }
            (Call::SetModel, Ok(_)) => {}
            (Call::Prompt, result) => {
                self.in_flight = false;
                if std::mem::take(&mut self.cancelled) {
                    // Settled when it was cancelled.
                } else {
                    self.prompt_finished(result, &mut output.deltas);
                }
                self.send_next_prompt(output);
            }
        }
    }

    /// `mcpServers` for the session requests. An agent that does not say it takes
    /// HTTP servers gets only the stdio ones, which every ACP agent must take.
    fn session_mcp_servers(&self, http: bool, deltas: &mut Vec<Delta>) -> Value {
        let pairs = |pairs: &[(String, String)]| -> Vec<Value> {
            pairs.iter().map(|(name, value)| json!({"name": name, "value": value})).collect()
        };
        let mut servers = Vec::new();
        for server in &self.mcp_servers {
            match &server.transport {
                McpTransport::Stdio(stdio) => servers.push(json!({
                    "name": server.name,
                    "command": stdio.command,
                    "args": stdio.args,
                    "env": pairs(&stdio.env),
                })),
                McpTransport::Http(http_server) if http => servers.push(json!({
                    "type": "http",
                    "name": server.name,
                    "url": http_server.url,
                    "headers": pairs(&http_server.headers),
                })),
                McpTransport::Http(_) => deltas.push(Delta::ProviderWarning(format!(
                    "the agent does not take HTTP MCP servers; {} is left out",
                    server.name
                ))),
            }
        }
        Value::Array(servers)
    }

    fn new_session(&mut self) -> String {
        self.peer.request(
            "session/new",
            json!({"cwd": self.cwd, "mcpServers": self.mcp_param}),
            Call::NewSession,
        )
    }

    fn session_ready(&mut self, session: String, output: &mut Output) {
        if let Some(model) = &self.model {
            output.writes.push(self.peer.request(
                "session/set_config_option",
                json!({"sessionId": session, "configId": "model", "value": model}),
                Call::SetModel,
            ));
        }
        self.session = Some(session);
        self.send_next_prompt(output);
    }

    fn prompt_finished(&mut self, result: Result<Value, RpcError>, deltas: &mut Vec<Delta>) {
        let result = match result {
            Ok(result) => result,
            Err(error) => return self.settle(TurnStatus::Failed, Some(error.message), deltas),
        };
        // The answer is complete; close it before the turn's bookkeeping.
        if let Some(stream) = self.stream.take() {
            deltas.push(Delta::ItemTextClose(TextClose {
                key: stream.key(),
                channel: stream.channel(),
                text: None,
            }));
        }
        if let Some(usage) = result.get("usage") {
            let tokens = |name: &str| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            let cached = usage.get("cachedReadTokens").and_then(Value::as_u64);
            let last = TokenUsage {
                input_tokens: tokens("inputTokens"),
                output_tokens: tokens("outputTokens"),
                cached_input_tokens: cached,
            };
            // ACP reports per prompt; the session total is ours to keep.
            add_usage(&mut self.total, &last);
            deltas.push(Delta::Usage(UsageSnapshot {
                total: self.total,
                last,
                model_context_window: self.context_window,
            }));
        }
        match str_field(&result, "stopReason") {
            "end_turn" | "" => self.settle(TurnStatus::Completed, None, deltas),
            "cancelled" => self.settle(TurnStatus::Interrupted, None, deltas),
            reason => self.settle(
                TurnStatus::Failed,
                Some(format!("the agent stopped: {reason}")),
                deltas,
            ),
        }
    }

    fn settle(&mut self, status: TurnStatus, error: Option<String>, deltas: &mut Vec<Delta>) {
        self.stream = None;
        self.tools.clear();
        self.closed.clear();
        self.approvals.clear();
        self.refused.clear();
        if std::mem::take(&mut self.turn_open) {
            deltas.push(boundary(status, error));
        }
    }

    fn update(&mut self, update: &Value, deltas: &mut Vec<Delta>) {
        // Outside a turn there is only replayed history and the tail of a cancelled
        // prompt.
        if self.replaying || !self.turn_open {
            return;
        }
        match str_field(update, "sessionUpdate") {
            "agent_message_chunk" => self.chunk(Stream::Message, update, deltas),
            "agent_thought_chunk" => self.chunk(Stream::Thought, update, deltas),
            "tool_call" | "tool_call_update" => self.tool_update(update, deltas),
            "plan" => {
                let plan: Vec<String> = update["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|entry| {
                        let mark = match str_field(entry, "status") {
                            "completed" => "x",
                            "in_progress" => "~",
                            _ => " ",
                        };
                        format!("[{mark}] {}", str_field(entry, "content"))
                    })
                    .collect();
                if !plan.is_empty() {
                    // Each update is the whole plan: a new snapshot item every time.
                    deltas.push(Delta::ItemTextClose(TextClose {
                        key: ItemKey::Anonymous("plan".into()),
                        channel: TextChannel::Plan,
                        text: Some(plan.join("\n")),
                    }));
                }
            }
            "usage_update" => {
                if let Some(size) = update.get("size").and_then(Value::as_u64) {
                    self.context_window = Some(size);
                }
            }
            // Catalogues and echoes the timeline has no use for.
            "available_commands_update"
            | "current_mode_update"
            | "config_option_update"
            | "session_info_update"
            | "user_message_chunk" => {}
            _ => deltas.push(Delta::Unhandled(update.to_string())),
        }
    }

    fn chunk(&mut self, stream: Stream, update: &Value, deltas: &mut Vec<Delta>) {
        let text = str_field(&update["content"], "text");
        if text.is_empty() {
            return;
        }
        // The agent switches between thinking and answering without saying so.
        if let Some(previous) = self.stream.filter(|previous| *previous != stream) {
            deltas.push(Delta::ItemTextClose(TextClose {
                key: previous.key(),
                channel: previous.channel(),
                text: None,
            }));
        }
        self.stream = Some(stream);
        deltas.push(Delta::ItemTextDelta(TextDelta {
            key: stream.key(),
            channel: stream.channel(),
            text: text.to_owned(),
        }));
    }

    fn tool_update(&mut self, update: &Value, deltas: &mut Vec<Delta>) {
        let id = str_field(update, "toolCallId").to_owned();
        if id.is_empty() || self.closed.contains(&id) {
            return;
        }
        let tool = self.tools.entry(id.clone()).or_default();
        tool.merge(update);
        let key = ItemKey::Provider(id.clone());
        let status = match tool.status.as_str() {
            "completed" => Some(ItemStatus::Completed),
            "failed" if self.refused.contains(&id) => Some(ItemStatus::Declined),
            "failed" => Some(ItemStatus::Failed),
            _ => None,
        };
        match status {
            Some(status) => {
                deltas.push(Delta::ItemClose(ItemClose { key, shape: tool.shape(), status }));
                self.tools.remove(&id);
                self.closed.insert(id);
            }
            None if !tool.opened && tool.has_details() => {
                tool.opened = true;
                // The core ends the text stream above a tool call; so do we.
                self.stream = None;
                deltas.push(Delta::ItemOpen(ItemOpen { key, shape: tool.shape() }));
            }
            None => {}
        }
    }

    fn permission(&mut self, id: Value, params: &Value, output: &mut Output) {
        if !self.turn_open {
            let cancelled = json!({"outcome": {"outcome": "cancelled"}});
            output.writes.push(self.peer.respond(&id, cancelled));
            return;
        }
        let call = &params["toolCall"];
        let tool_id = str_field(call, "toolCallId").to_owned();
        // The request often knows more about the call than the updates so far did.
        let tool = self.tools.entry(tool_id.clone()).or_default();
        tool.merge(call);
        if !tool.opened {
            tool.opened = true;
            self.stream = None;
            output.deltas.push(Delta::ItemOpen(ItemOpen {
                key: ItemKey::Provider(tool_id.clone()),
                shape: tool.shape(),
            }));
        }

        // The permission mode may answer for the user.
        let approved = match self.mode {
            Some(PermissionMode::Full | PermissionMode::Auto) => true,
            Some(PermissionMode::AcceptEdits) => {
                matches!(tool.kind.as_str(), "edit" | "delete" | "move")
            }
            Some(PermissionMode::Ask) | None => false,
        };
        let allow = ["allow_once", "allow_always"].iter().find_map(|kind| {
            params["options"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|offered| str_field(offered, "kind") == *kind)
                .map(|offered| str_field(offered, "optionId").to_owned())
        });
        if let (true, Some(option)) = (approved, allow) {
            let outcome = json!({"outcome": {"outcome": "selected", "optionId": option}});
            output.writes.push(self.peer.respond(&id, outcome));
            return;
        }

        let mut options = Vec::new();
        let mut rejecting = Vec::new();
        for offered in params["options"].as_array().into_iter().flatten() {
            let option_id = str_field(offered, "optionId").to_owned();
            let kind = match str_field(offered, "kind") {
                "allow_once" | "allow_always" => OptionKind::Allow,
                "reject_once" | "reject_always" => {
                    rejecting.push(option_id.clone());
                    OptionKind::Deny
                }
                _ => OptionKind::Other,
            };
            options.push(InteractionOption {
                id: option_id,
                label: str_field(offered, "name").to_owned(),
                kind,
            });
        }
        let request_id = id.to_string();
        output.deltas.push(Delta::InteractionRequest(InteractionAsk {
            request_id: request_id.clone(),
            item: Some(ItemKey::Provider(tool_id.clone())),
            request: InteractionRequest::Approval(ApprovalRequest {
                title: format!("Allow {}?", if tool.kind.is_empty() { "this" } else { &tool.kind }),
                detail: Some(str_field(call, "title").to_owned()).filter(|title| !title.is_empty()),
                options: options.clone(),
            }),
        }));
        let offered = options.into_iter().map(|option| option.id).collect();
        self.approvals.insert(request_id, Approval { id, tool: tool_id, rejecting, offered });
    }
}

impl LineBridge for Bridge {
    /// One line of the agent's stdout.
    fn on_line(&mut self, line: &str) -> Output {
        let mut output = Output::default();
        match self.peer.classify(line) {
            Incoming::Response { tag, result } => self.response(tag, result, &mut output),
            Incoming::Notification { method, params } => {
                if method == "session/update" {
                    self.update(&params["update"], &mut output.deltas);
                }
            }
            Incoming::Request { id, method, params } => {
                if method == "session/request_permission" {
                    self.permission(id, &params, &mut output);
                } else {
                    output.writes.push(self.peer.refuse(&id, &method));
                }
            }
            Incoming::Unknown(_) => {}
        }
        output
    }

    /// A prompt still waiting for the agent is lost with it, which is an error too.
    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta> {
        let turn_open = std::mem::take(&mut self.turn_open);
        let error_if_idle = code != Some(0) || !self.waiting.is_empty();
        stdio::exited(self.agent, turn_open, error_if_idle, code, stderr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config { agent: "agent", cwd: "/work".into(), ..Config::default() }
    }

    const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}}"#;
    const INIT_LOAD_ONLY: &str =
        r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"loadSession":true}}}"#;
    const INIT_BARE: &str = r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{}}}"#;

    fn method(line: &str) -> String {
        let value: Value = serde_json::from_str(line).unwrap();
        value["method"].as_str().unwrap_or("response").to_owned()
    }

    fn methods(output: &Output) -> Vec<String> {
        output.writes.iter().map(|line| method(line)).collect()
    }

    fn chunk(kind: &str, text: &str) -> String {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"update": {
            "sessionUpdate": kind, "content": {"type": "text", "text": text}}}})
        .to_string()
    }

    /// A bridge with a ready session `s` and a running turn (prompt request id 3).
    fn running() -> Bridge {
        let (mut bridge, _) = Bridge::new(config());
        bridge.on_line(INIT);
        bridge.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        assert_eq!(bridge.prompt("go").deltas, [Delta::TurnOpen]);
        bridge
    }

    #[test]
    fn a_prompt_waits_for_the_handshake() {
        let (mut bridge, handshake) = Bridge::new(Config { model: Some("m1".into()), ..config() });
        assert_eq!(methods(&handshake), ["initialize"]);
        // Nothing is sent and no turn opens before the agent has a session.
        assert_eq!(bridge.prompt("hello"), Output::default());
        assert_eq!(methods(&bridge.on_line(INIT)), ["session/new"]);

        let ready = bridge.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        assert_eq!(methods(&ready), ["session/set_config_option", "session/prompt"]);
        assert_eq!(ready.deltas, [Delta::SessionIdentity("s1".into()), Delta::TurnOpen]);
        assert!(ready.writes[0].contains("\"value\":\"m1\""));
        assert!(ready.writes[1].contains("\"sessionId\":\"s1\""));

        // A model the agent does not know is a warning, not a failure.
        let refused = bridge.on_line(
            r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"model not found"}}"#,
        );
        assert_eq!(
            refused.deltas,
            [Delta::ProviderWarning("could not select the model: model not found".into())]
        );
    }

    #[test]
    fn continuing_prefers_resume_then_load_then_a_new_session() {
        let resume = || Bridge::new(Config { resume: Some("old".into()), ..config() }).0;

        let mut bridge = resume();
        let request = bridge.on_line(INIT);
        assert_eq!(methods(&request), ["session/resume"]);
        assert!(request.writes[0].contains("\"sessionId\":\"old\""));
        // A resumed session is already known to the core: nothing to announce.
        assert_eq!(bridge.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#), Output::default());
        assert_eq!(methods(&bridge.prompt("hi")), ["session/prompt"]);

        let mut bridge = resume();
        assert_eq!(methods(&bridge.on_line(INIT_LOAD_ONLY)), ["session/load"]);
        // `load` replays the conversation; none of it is new.
        assert_eq!(bridge.on_line(&chunk("agent_message_chunk", "old answer")), Output::default());
        bridge.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
        bridge.prompt("hi");
        assert_eq!(bridge.on_line(&chunk("agent_message_chunk", "new")).deltas.len(), 1);

        let mut bridge = resume();
        let fallback = bridge.on_line(INIT_BARE);
        assert_eq!(methods(&fallback), ["session/new"]);
        assert!(matches!(fallback.deltas[..], [Delta::ProviderWarning(_)]));

        // The agent no longer has the session: say so and start over.
        let mut bridge = resume();
        bridge.on_line(INIT);
        let gone = bridge
            .on_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32002,"message":"not found"}}"#);
        assert_eq!(methods(&gone), ["session/new"]);
        assert!(matches!(&gone.deltas[..], [Delta::ProviderWarning(w)] if w.contains("not found")));
        let fresh = bridge.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"new"}}"#);
        assert_eq!(fresh.deltas, [Delta::SessionIdentity("new".into())]);
    }

    #[test]
    fn mcp_servers_go_with_every_session_request() {
        let servers = vec![
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
        let docs = json!({"name": "docs", "command": "npx", "args": ["docs-mcp"],
                          "env": [{"name": "TOKEN", "value": "t"}]});
        let request = |line: &str| -> Value { serde_json::from_str(line).unwrap() };

        // An agent without HTTP support gets the stdio server and a warning.
        let (mut bridge, _) = Bridge::new(Config { mcp_servers: servers.clone(), ..config() });
        let new = bridge.on_line(INIT);
        assert_eq!(request(&new.writes[0])["params"]["mcpServers"], json!([docs]));
        assert!(matches!(&new.deltas[..], [Delta::ProviderWarning(w)] if w.contains("web")));

        // One that takes HTTP gets both, on a resumed session too.
        let init = r#"{"jsonrpc":"2.0","id":1,"result":{"agentCapabilities":{"sessionCapabilities":{"resume":{}},"mcpCapabilities":{"http":true}}}}"#;
        let (mut bridge, _) =
            Bridge::new(Config { resume: Some("s".into()), mcp_servers: servers, ..config() });
        let resume = bridge.on_line(init);
        assert_eq!(method(&resume.writes[0]), "session/resume");
        assert_eq!(
            request(&resume.writes[0])["params"]["mcpServers"],
            json!([docs, {"type": "http", "name": "web", "url": "https://example.com/mcp",
                          "headers": [{"name": "Authorization", "value": "Bearer x"}]}])
        );
        assert!(resume.deltas.is_empty());
    }

    #[test]
    fn a_refused_handshake_ends_the_session() {
        let (mut bridge, _) = Bridge::new(config());
        bridge.prompt("hello");
        let refused = bridge.on_line(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}"#);
        assert!(refused.fatal);
        assert!(matches!(
            &refused.deltas[..],
            [Delta::ProviderError(e), Delta::SessionEnded(Some(_))] if e.contains("Authentication required")
        ));
    }

    #[test]
    fn thought_and_message_are_separate_items() {
        let mut bridge = running();
        let thought = bridge.on_line(&chunk("agent_thought_chunk", "hm"));
        assert!(
            matches!(&thought.deltas[..], [Delta::ItemTextDelta(d)] if d.channel == TextChannel::Reasoning)
        );
        let answer = bridge.on_line(&chunk("agent_message_chunk", "yes"));
        assert!(matches!(
            &answer.deltas[..],
            [Delta::ItemTextClose(c), Delta::ItemTextDelta(d)]
                if c.channel == TextChannel::Reasoning && d.channel == TextChannel::AgentMessage
        ));
        assert_eq!(bridge.on_line(&chunk("agent_message_chunk", "")), Output::default());
    }

    #[test]
    fn queued_prompts_run_one_at_a_time() {
        let mut bridge = running();
        assert_eq!(bridge.prompt("second"), Output::default(), "held while a prompt is in flight");
        let done = bridge.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn","usage":{"inputTokens":5,"outputTokens":1,"cachedReadTokens":2}}}"#);
        assert_eq!(methods(&done), ["session/prompt"]);
        assert!(matches!(
            &done.deltas[..],
            [Delta::Usage(_), Delta::TurnBoundary(b), Delta::TurnOpen]
                if b.status == TurnStatus::Completed
        ));
        let failed =
            bridge.on_line(r#"{"jsonrpc":"2.0","id":4,"result":{"stopReason":"max_tokens"}}"#);
        assert!(matches!(
            &failed.deltas[..],
            [Delta::TurnBoundary(b)] if b.status == TurnStatus::Failed
                && b.error.as_deref() == Some("the agent stopped: max_tokens")
        ));
    }

    #[test]
    fn cancel_settles_now_and_holds_the_next_prompt_until_the_agent_caught_up() {
        let mut bridge = running();
        let permission = bridge.on_line(r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"toolCall":{"toolCallId":"t","kind":"execute","title":"rm -rf x"},"options":[{"optionId":"once","kind":"allow_once","name":"Allow"}]}}"#);
        assert!(matches!(
            &permission.deltas[..],
            [Delta::ItemOpen(_), Delta::InteractionRequest(_)]
        ));

        let cancel = bridge.interrupt();
        assert_eq!(methods(&cancel), ["session/cancel", "response"]);
        assert!(cancel.writes[1].contains("\"outcome\":\"cancelled\""));
        assert!(
            matches!(&cancel.deltas[..], [Delta::TurnBoundary(b)] if b.status == TurnStatus::Interrupted)
        );
        assert_eq!(bridge.interrupt(), Output::default());

        assert_eq!(bridge.prompt("next"), Output::default());
        assert_eq!(bridge.on_line(&chunk("agent_message_chunk", "late")), Output::default());
        let caught_up =
            bridge.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}"#);
        assert_eq!(methods(&caught_up), ["session/prompt"]);
        assert_eq!(caught_up.deltas, [Delta::TurnOpen]);
    }

    #[test]
    fn the_permission_mode_answers_for_the_user() {
        let ask = |kind: &str| {
            json!({"jsonrpc": "2.0", "id": 5, "method": "session/request_permission", "params": {
            "toolCall": {"toolCallId": "t", "kind": kind, "title": "x"},
            "options": [
                {"optionId": "always", "kind": "allow_always", "name": "Always"},
                {"optionId": "once", "kind": "allow_once", "name": "Once"},
                {"optionId": "no", "kind": "reject_once", "name": "No"},
            ]}})
            .to_string()
        };
        let with_mode = |mode| {
            let (mut bridge, _) = Bridge::new(Config { mode, ..config() });
            bridge.on_line(INIT);
            bridge.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
            bridge.prompt("go");
            bridge
        };
        let answered = |output: &Output| {
            // Allowed once, never "always": a mode is not a standing rule in the agent.
            output.writes.len() == 1
                && output.writes[0].contains("\"optionId\":\"once\"")
                && !output.deltas.iter().any(|d| matches!(d, Delta::InteractionRequest(_)))
        };
        let asked = |output: &Output| {
            output.writes.is_empty()
                && output.deltas.iter().any(|d| matches!(d, Delta::InteractionRequest(_)))
        };

        assert!(answered(&with_mode(Some(PermissionMode::Full)).on_line(&ask("execute"))));
        let mut edits = with_mode(Some(PermissionMode::AcceptEdits));
        assert!(answered(&edits.on_line(&ask("edit"))));
        assert!(asked(&edits.on_line(&ask("execute"))));
        assert!(asked(&with_mode(Some(PermissionMode::Ask)).on_line(&ask("edit"))));
        assert!(asked(&with_mode(None).on_line(&ask("edit"))));
    }

    #[test]
    fn requests_we_do_not_serve_are_refused() {
        let mut bridge = running();
        let refused =
            bridge.on_line(r#"{"jsonrpc":"2.0","id":1,"method":"fs/write_text_file","params":{}}"#);
        assert!(refused.writes[0].contains("-32601"));
        assert!(refused.deltas.is_empty());
        assert!(bridge.resolve("nope", &InteractionResolution::Cancelled).is_err());
    }
}
