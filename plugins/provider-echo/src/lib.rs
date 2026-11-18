//! The smallest provider: it echoes prompts back.
//!
//! It doubles as the daemon's test provider, so a few prompts exercise the rest of the
//! plugin contract:
//!
//! | prompt               | behaviour                                                    |
//! | -------------------- | ------------------------------------------------------------ |
//! | anything else        | streams the prompt back word by word                         |
//! | `/slow <text>`       | streams one word per timer tick; can be interrupted          |
//! | `/tool <name>`       | a tool call that opens and closes; `server__tool` is MCP     |
//! | `/ask <question>`    | asks the user a question and echoes the answer               |
//! | `/approve <command>` | asks for approval of a command item (not in `auto`/`full`)   |
//! | `/exec <prog> <args>`| runs a process and streams its stdout                        |
//! | `/kv`                | counts invocations in the plugin's key-value store           |
//! | `/card <text>`       | an item of the plugin's own kind, drawn by the plugin        |
//! | `/fail <message>`    | the turn fails                                               |
//! | `/skills`            | says what the session was told (skills, project folders)     |
//! | `/mcp`               | lists the MCP servers the session was given                  |
//! | `/agent`             | says the model and effort the session was started with       |
//! | `/notes`             | says what qcc told it along with the message                 |
//! | `/attachments`       | lists the files attached to the message and how each came    |
//! | `/settings`          | lists the plugin's own settings as the daemon told them      |
//! | `/context`           | reports a made-up breakdown of its context window            |
//! | `/limits <percent>`  | reports its account's 5-hour window spent that much (`42`)   |
//! | `/crash`             | the plugin traps                                             |
//! | `/spin`              | a timer that restarts itself with no delay until interrupted |
//!
//! A message steered into a running `/slow` turn is appended to what it streams.
//!
//! Each MCP server it is given is reported ready at once, or failed when its
//! command or URL contains `fail`. It changes the servers of a running session
//! (`mcp-live`): a new one or one set up anew is reported again, one dropped is
//! removed.
//!
//! It offers a skill on request, `echo-demo`, for tests of skill switching, and
//! compacts a session's context when asked, at once. Asked for its account's usage
//! limits without a session, it reports a 5-hour window 10% spent and a weekly one
//! 20% spent.
//!
//! It also contributes UI: an "Echo" tab on its own threads and the drawing of its
//! `/card` items, both built from the fixed set of UI nodes.

use std::collections::{HashMap, VecDeque};

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::ui;

/// The extension item `/card` makes. A kind is always `<plugin-id>/<name>`.
const CARD_KIND: &str = "provider-echo/card";
const PLAYGROUND_VIEW: &str = "playground";
/// A view that is a page of the plugin's web bundle instead of UI nodes.
const CANVAS_VIEW: &str = "canvas";

const SLOW_TICK_MS: u64 = 10;

#[derive(Default)]
struct Echo {
    sessions: HashMap<String, Session>,
    /// Its own settings, as the daemon last told them.
    settings: Vec<(String, String)>,
}

/// How `/mcp` lists a server.
fn mcp_line(server: &McpServer) -> String {
    match &server.transport {
        McpTransport::Stdio(stdio) => {
            format!("{} stdio {} {}", server.name, stdio.command, stdio.args.join(" "))
        }
        McpTransport::Http(http) => format!("{} http {}", server.name, http.url),
    }
}

/// How a server the session was given is doing.
fn mcp_status(server: &McpServer) -> McpStatus {
    let target = match &server.transport {
        McpTransport::Stdio(stdio) => &stdio.command,
        McpTransport::Http(http) => &http.url,
    };
    let failed = target.contains("fail");
    McpStatus {
        name: server.name.clone(),
        state: if failed { McpState::Failed } else { McpState::Ready },
        error: failed.then(|| format!("echo was told to fail {target}")),
    }
}

#[derive(Default)]
struct Session {
    input_tokens: u64,
    output_tokens: u64,
    turn: Option<Turn>,
    next_request: u64,
    mode: Option<PermissionMode>,
    /// What a real bridge would hand its agent: skills and the project's folders.
    skills: Option<String>,
    /// The model and effort it was started with, in a line.
    agent: String,
    /// The MCP servers it was given, at the start or since.
    mcp_servers: Vec<McpServer>,
}

/// What the running turn is waiting for.
enum Turn {
    Slow {
        words: VecDeque<String>,
        timer: u64,
    },
    /// Busy on timers alone: each one starts the next with no delay.
    Spin {
        timer: u64,
    },
    Asking {
        request_id: String,
    },
    Approving {
        request_id: String,
        command: String,
    },
    Exec {
        process: u64,
    },
}

fn assistant() -> ItemKey {
    ItemKey::Anonymous("assistant".into())
}

fn text(text: impl Into<String>) -> Delta {
    Delta::ItemTextDelta(TextDelta {
        key: assistant(),
        channel: TextChannel::AgentMessage,
        text: text.into(),
    })
}

fn text_close() -> Delta {
    Delta::ItemTextClose(TextClose {
        key: assistant(),
        channel: TextChannel::AgentMessage,
        text: None,
    })
}

fn completed() -> Delta {
    boundary(TurnStatus::Completed, None)
}

fn command_item(command: &str, exit_code: Option<i32>) -> ItemShape {
    ItemShape::Command(CommandItem {
        command: command.to_owned(),
        cwd: None,
        output: None,
        exit_code,
    })
}

fn words(text: &str) -> u64 {
    text.split_whitespace().count() as u64
}

impl Session {
    fn usage(&mut self, input: &str, output: &str) -> Delta {
        let last = TokenUsage {
            input_tokens: words(input),
            output_tokens: words(output),
            cached_input_tokens: None,
        };
        self.input_tokens += last.input_tokens;
        self.output_tokens += last.output_tokens;
        let total = TokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_input_tokens: None,
        };
        let context_used = Some(last.input_tokens);
        Delta::Usage(UsageSnapshot { total, last, model_context_window: None, context_used })
    }

    /// A breakdown of a made-up window: fixed parts, and the session's words as its
    /// messages.
    fn context(&self) -> ContextUsage {
        const WINDOW: u64 = 10_000;
        let messages = self.input_tokens + self.output_tokens;
        let category = |id: &str, name: &str, tokens: u64, kind: ContextKind| ContextCategory {
            id: id.into(),
            name: name.into(),
            tokens,
            kind,
        };
        let used = 1_000 + 500 + 200 + messages;
        ContextUsage {
            used,
            window: Some(WINDOW),
            estimated: false,
            categories: vec![
                category("system_prompt", "System prompt", 1_000, ContextKind::Used),
                category("system_tools", "System tools", 500, ContextKind::Used),
                category("skills", "Skills", 200, ContextKind::Used),
                category("messages", "Messages", messages, ContextKind::Used),
                category("autocompact_buffer", "Autocompact buffer", 1_000, ContextKind::Buffer),
                category(
                    "free_space",
                    "Free space",
                    WINDOW.saturating_sub(used + 1_000),
                    ContextKind::Free,
                ),
            ],
            entries: vec![ContextEntry {
                category: "skills".into(),
                name: "echo-demo".into(),
                tokens: 200,
            }],
        }
    }

    fn request_id(&mut self) -> String {
        self.next_request += 1;
        format!("echo-request-{}", self.next_request)
    }
}

/// One line of `/attachments`: what the file is and whether its bytes came along.
fn describe_attachment(attachment: &Attachment) -> String {
    let how = match &attachment.data {
        Some(data) => format!("image, {} bytes inline", data.len()),
        None => "by path".to_owned(),
    };
    format!(
        "{} ({}, {} bytes) {how}: {}",
        attachment.name, attachment.mime, attachment.size, attachment.path
    )
}

impl Echo {
    /// Run one prompt. Opens a turn and either settles it or leaves it waiting.
    fn run(
        &mut self,
        thread: &str,
        prompt: &str,
        notes: &[String],
        attachments: &[Attachment],
    ) -> Result<(), String> {
        let session = self.sessions.get_mut(thread).ok_or("the session was not started")?;
        if session.turn.is_some() {
            return Err("a turn is already running".into());
        }
        let (command, rest) = match prompt.trim().split_once(char::is_whitespace) {
            Some((command, rest)) => (command, rest.trim()),
            None => (prompt.trim(), ""),
        };
        let mut out = vec![Delta::TurnOpen];
        match command {
            "/slow" => {
                let words = rest.split_whitespace().map(str::to_owned).collect();
                let timer = timers::start(SLOW_TICK_MS)?;
                session.turn = Some(Turn::Slow { words, timer });
            }
            "/spin" => {
                let timer = timers::start(0)?;
                session.turn = Some(Turn::Spin { timer });
            }
            "/tool" => {
                let key = ItemKey::Provider(format!("call-{}", session.request_id()));
                let tool = |output: Option<String>| {
                    ItemShape::Tool(ToolItem {
                        name: rest.to_owned(),
                        input: "{}".into(),
                        output,
                        // `/tool server__name` is a tool of an MCP server.
                        mcp_server: rest.split_once("__").map(|(server, _)| server.to_owned()),
                        skill: None,
                    })
                };
                out.push(text("Calling a tool."));
                out.push(Delta::ItemOpen(ItemOpen { key: key.clone(), shape: tool(None) }));
                out.push(Delta::ItemClose(ItemClose {
                    key,
                    shape: tool(Some(format!("{rest} ok"))),
                    status: ItemStatus::Completed,
                }));
                out.push(text("Done."));
                out.push(text_close());
                out.push(completed());
            }
            "/card" => {
                let key = ItemKey::Provider(format!("card-{}", session.request_id()));
                let payload = serde_json::json!({ "title": "A card from echo", "body": rest });
                let shape = ItemShape::Extension(ExtensionItem {
                    kind: CARD_KIND.into(),
                    payload: payload.to_string(),
                });
                out.push(Delta::ItemOpen(ItemOpen { key: key.clone(), shape: shape.clone() }));
                out.push(Delta::ItemClose(ItemClose { key, shape, status: ItemStatus::Completed }));
                out.push(completed());
            }
            "/ask" => {
                let request_id = session.request_id();
                out.push(Delta::InteractionRequest(InteractionAsk {
                    request_id: request_id.clone(),
                    item: None,
                    request: InteractionRequest::Question(QuestionRequest {
                        question: rest.to_owned(),
                        options: vec![
                            option("yes", "Yes", OptionKind::Other),
                            option("no", "No", OptionKind::Other),
                        ],
                        allow_text: true,
                    }),
                }));
                session.turn = Some(Turn::Asking { request_id });
            }
            "/approve"
                if matches!(session.mode, Some(PermissionMode::Auto | PermissionMode::Full)) =>
            {
                // The mode says not to ask: the command just runs.
                let key = ItemKey::Provider(session.request_id());
                out.push(Delta::ItemOpen(ItemOpen {
                    key: key.clone(),
                    shape: command_item(rest, None),
                }));
                out.push(Delta::ItemClose(ItemClose {
                    key,
                    shape: command_item(rest, Some(0)),
                    status: ItemStatus::Completed,
                }));
                out.push(completed());
            }
            "/approve" => {
                let request_id = session.request_id();
                let key = ItemKey::Provider(request_id.clone());
                out.push(Delta::ItemOpen(ItemOpen {
                    key: key.clone(),
                    shape: command_item(rest, None),
                }));
                out.push(Delta::InteractionRequest(InteractionAsk {
                    request_id: request_id.clone(),
                    item: Some(key),
                    request: InteractionRequest::Approval(ApprovalRequest {
                        title: "Run command?".into(),
                        detail: Some(rest.to_owned()),
                        options: vec![
                            option("allow", "Allow", OptionKind::Allow),
                            option("deny", "Deny", OptionKind::Deny),
                        ],
                    }),
                }));
                session.turn = Some(Turn::Approving { request_id, command: rest.to_owned() });
            }
            "/exec" => {
                let mut args = rest.split_whitespace().map(str::to_owned);
                let program = args.next().unwrap_or_default();
                let spawned = process::spawn(&process::SpawnOptions {
                    program,
                    args: args.collect(),
                    env: Vec::new(),
                    cwd: None,
                });
                match spawned {
                    Ok(process) => {
                        // Nothing more to say on stdin.
                        process::close_stdin(process)?;
                        session.turn = Some(Turn::Exec { process });
                    }
                    Err(error) => out.push(boundary(TurnStatus::Failed, Some(error))),
                }
            }
            "/kv" => {
                let count = match kv::get("count")? {
                    Some(bytes) => String::from_utf8_lossy(&bytes).parse::<u64>().unwrap_or(0),
                    None => 0,
                } + 1;
                kv::put("count", count.to_string().as_bytes())?;
                let reply = format!("count {count}");
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/fail" => out.push(boundary(TurnStatus::Failed, Some(rest.to_owned()))),
            "/mcp" => {
                let reply = if session.mcp_servers.is_empty() {
                    "no MCP servers".to_owned()
                } else {
                    session.mcp_servers.iter().map(mcp_line).collect::<Vec<_>>().join("\n")
                };
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/crash" => panic!("echo provider was asked to crash"),
            "/settings" => {
                let reply = if self.settings.is_empty() {
                    "no settings".to_owned()
                } else {
                    let lines: Vec<String> =
                        self.settings.iter().map(|(id, value)| format!("{id}={value}")).collect();
                    lines.join("\n")
                };
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/notes" => {
                let reply = if notes.is_empty() { "no notes".into() } else { notes.join("\n") };
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/attachments" => {
                let reply = if attachments.is_empty() {
                    "no attachments".to_owned()
                } else {
                    let lines: Vec<String> = attachments.iter().map(describe_attachment).collect();
                    lines.join("\n")
                };
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/agent" => {
                let reply = session.agent.clone();
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            "/context" => {
                let reply = "context reported".to_owned();
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(Delta::ContextUsage(session.context()));
                out.push(completed());
            }
            "/limits" => {
                let used = rest.trim().parse().unwrap_or(42);
                let reply = format!("limits reported: 5-hour {used}%");
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                // Only the window the turn spent, as a real agent's rate-limit event
                // reports it: the others are left as they were.
                let window = limit_window("five_hour", "5-hour", 300, used);
                let limits = RateLimits { plan: None, windows: vec![window], complete: false };
                out.push(Delta::RateLimits(limits));
                out.push(completed());
            }
            "/skills" => {
                let reply = session.skills.clone().unwrap_or_else(|| "no skills".into());
                out.push(text(reply.clone()));
                out.push(text_close());
                out.push(session.usage(prompt, &reply));
                out.push(completed());
            }
            _ => {
                for (index, word) in prompt.split_whitespace().enumerate() {
                    out.push(text(if index == 0 { word.to_owned() } else { format!(" {word}") }));
                }
                out.push(text_close());
                out.push(session.usage(prompt, prompt));
                out.push(completed());
            }
        }
        deltas::emit(thread, &out)
    }

    /// Stop whatever the running turn waits for and settle it as interrupted.
    fn interrupt_turn(&mut self, thread: &str) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(thread) else {
            return Ok(());
        };
        match session.turn.take() {
            None => return Ok(()),
            Some(Turn::Slow { timer, .. } | Turn::Spin { timer }) => timers::cancel(timer),
            Some(Turn::Exec { process }) => {
                // The process may have exited already; its exit event is ignored below
                // because no turn refers to it any more.
                let _ = process::kill(process);
            }
            Some(Turn::Asking { .. } | Turn::Approving { .. }) => {}
        }
        deltas::emit(thread, &[boundary(TurnStatus::Interrupted, None)])
    }

    fn thread_of_process(&self, process: u64) -> Option<String> {
        self.sessions.iter().find_map(|(thread, session)| match session.turn {
            Some(Turn::Exec { process: running }) if running == process => Some(thread.clone()),
            _ => None,
        })
    }
}

impl Provider for Echo {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "echo".into(),
            name: "Echo".into(),
            capabilities: ProviderCapabilities {
                resume: true,
                steer: true,
                permission_modes: vec![
                    PermissionMode::Ask,
                    PermissionMode::AcceptEdits,
                    PermissionMode::Auto,
                    PermissionMode::Full,
                ],
                mcp_live: true,
                compact: true,
                list_skills: true,
                context_usage: true,
                images: true,
                rate_limits: true,
            },
        }
    }

    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
        let session = Session {
            mode: start.permission_mode,
            skills: session_context(&start),
            agent: format!(
                "model {}, effort {}",
                start.model.as_deref().unwrap_or("default"),
                start.effort.as_deref().unwrap_or("default")
            ),
            mcp_servers: start.mcp_servers.clone(),
            ..Session::default()
        };
        self.sessions.insert(start.thread.clone(), session);
        // Every server it is given connects at once, but one whose command or URL
        // says `fail` fails: what a test needs to see both.
        let mut reset = vec![Delta::SessionReset];
        reset.extend(start.mcp_servers.iter().map(|server| Delta::McpStatus(mcp_status(server))));
        deltas::emit(&start.thread, &reset)?;
        if let Some(prompt) = &start.prompt {
            self.run(&start.thread, prompt, &start.prompt_notes, &start.prompt_attachments)?;
        }
        Ok(SessionStarted {
            provider_session: Some(start.resume.unwrap_or(format!("echo-{}", start.thread))),
        })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        self.run(&input.thread, &input.text, &input.notes, &input.attachments)
    }

    /// A turn that sums up nothing: the tokens so far go down to none.
    fn compact(&mut self, thread: String) -> Result<(), String> {
        let session = self.sessions.get_mut(&thread).ok_or("the session was not started")?;
        if session.turn.is_some() {
            return Err("a turn is running".into());
        }
        let before = session.input_tokens + session.output_tokens;
        session.input_tokens = 0;
        session.output_tokens = 0;
        let compaction =
            Compaction { automatic: false, tokens_before: Some(before), tokens_after: Some(0) };
        deltas::emit(&thread, &[Delta::TurnOpen, Delta::ContextCompacted(compaction), completed()])
    }

    fn set_mcp_servers(&mut self, update: McpServersUpdate) -> Result<(), String> {
        let session = self.sessions.get_mut(&update.thread).ok_or("the session was not started")?;
        let before = std::mem::replace(&mut session.mcp_servers, update.servers.clone());
        let removed = before
            .iter()
            .filter(|old| !update.servers.iter().any(|server| server.name == old.name))
            .map(|old| Delta::McpRemoved(old.name.clone()));
        let changed = update
            .servers
            .iter()
            .filter(|server| !before.contains(server))
            .map(|server| Delta::McpStatus(mcp_status(server)));
        let out: Vec<Delta> = removed.chain(changed).collect();
        if out.is_empty() { Ok(()) } else { deltas::emit(&update.thread, &out) }
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let session = self.sessions.get_mut(&input.thread).ok_or("the session was not started")?;
        match &mut session.turn {
            Some(Turn::Slow { words, .. }) => {
                words.extend(input.text.split_whitespace().map(str::to_owned));
                Ok(())
            }
            Some(_) => Err("this turn is waiting for an answer, not for more input".into()),
            None => Err("no turn is running".into()),
        }
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        self.interrupt_turn(&thread)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        self.interrupt_turn(&thread)?;
        if self.sessions.remove(&thread).is_some() {
            deltas::emit(&thread, &[Delta::SessionEnded(Some("stopped".into()))])?;
        }
        Ok(())
    }

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let session = self.sessions.get_mut(&answer.thread).ok_or("the session was not started")?;
        let out = match session.turn.take() {
            Some(Turn::Asking { request_id }) if request_id == answer.request_id => {
                let reply = match &answer.resolution {
                    InteractionResolution::Selected(option) => format!("You chose {option}."),
                    InteractionResolution::Text(text) => format!("You said: {text}"),
                    InteractionResolution::Cancelled => "Never mind.".to_owned(),
                };
                vec![text(reply), text_close(), completed()]
            }
            Some(Turn::Approving { request_id, command }) if request_id == answer.request_id => {
                let allowed = matches!(&answer.resolution, InteractionResolution::Selected(id) if id == "allow");
                let (status, exit_code) = if allowed {
                    (ItemStatus::Completed, Some(0))
                } else {
                    (ItemStatus::Declined, None)
                };
                vec![
                    Delta::ItemClose(ItemClose {
                        key: ItemKey::Provider(request_id),
                        shape: command_item(&command, exit_code),
                        status,
                    }),
                    completed(),
                ]
            }
            other => {
                session.turn = other;
                return Err(format!("no interaction {:?} is pending", answer.request_id));
            }
        };
        deltas::emit(&answer.thread, &out)
    }

    fn models(&mut self) -> Result<Reply<Vec<ModelInfo>>, String> {
        let model = |id: &str, name: &str, is_default| ModelInfo {
            id: id.into(),
            name: name.into(),
            description: None,
            is_default,
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("low".into()),
        };
        Ok(Reply::Now(vec![model("echo-1", "Echo 1", true), model("echo-2", "Echo 2", false)]))
    }

    fn rate_limits(&mut self) -> Result<Reply<RateLimits>, String> {
        Ok(Reply::Now(RateLimits {
            plan: Some("echo".into()),
            windows: vec![
                limit_window("five_hour", "5-hour", 300, 10),
                limit_window("seven_day", "Weekly", 7 * 24 * 60, 20),
            ],
            complete: true,
        }))
    }

    /// One skill of its "own", as a real agent finds them in its folders.
    fn skills(&mut self) -> Result<Reply<Vec<AgentSkill>>, String> {
        Ok(Reply::Now(vec![AgentSkill {
            name: "echo-own".into(),
            description: "A skill the agent finds by itself.".into(),
            source: Some("user".into()),
            path: Some("/echo/skills/echo-own/SKILL.md".into()),
        }]))
    }
}

/// A usage window that started now, `used` percent spent.
fn limit_window(id: &str, label: &str, minutes: u64, used: u32) -> LimitWindow {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let status = match used {
        0..80 => LimitStatus::Allowed,
        80..100 => LimitStatus::Warning,
        _ => LimitStatus::Exceeded,
    };
    LimitWindow {
        id: id.into(),
        label: label.into(),
        scope: None,
        used_percent: used,
        resets_at: Some(now + minutes * 60_000),
        window_minutes: Some(minutes),
        status: Some(status),
    }
}

fn stored(key: &str) -> Result<String, String> {
    Ok(kv::get(key)?.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()).unwrap_or_default())
}

impl Ui for Echo {
    fn views(&self) -> Vec<UiViewInfo> {
        vec![
            UiViewInfo {
                id: PLAYGROUND_VIEW.into(),
                title: "Echo".into(),
                placement: UiPlacement::ThreadTab,
                scope: UiScope::OwnThreads,
                web: None,
            },
            UiViewInfo {
                id: CANVAS_VIEW.into(),
                title: "Echo canvas".into(),
                placement: UiPlacement::ThreadTab,
                scope: UiScope::OwnThreads,
                web: Some("index.html".into()),
            },
        ]
    }

    fn item_kinds(&self) -> Vec<String> {
        vec![CARD_KIND.into()]
    }

    fn render(&mut self, request: UiRender) -> Result<String, String> {
        if request.view != PLAYGROUND_VIEW {
            return Err(format!("unknown view {:?}", request.view));
        }
        let thread = request.thread.ok_or("this view belongs to a thread")?;
        let presses = stored(&format!("ui/presses/{thread}"))?;
        let note = stored(&format!("ui/note/{thread}"))?;
        let live = self.sessions.contains_key(&thread);
        let commands = [
            ("/slow <text>", "streams one word per tick; can be interrupted"),
            ("/tool <name>", "a tool call"),
            ("/ask <question>", "asks a question"),
            ("/approve <command>", "asks for approval"),
            ("/exec echo <args>", "runs a process"),
            ("/card <text>", "an item this plugin draws itself"),
            ("/fail <message>", "the turn fails"),
        ];
        let tree = ui::stack([
            ui::heading("Echo playground"),
            ui::text("This tab is drawn by the echo plugin from the app's UI nodes.")
                .tone(ui::Tone::Muted),
            ui::key_values([
                ("Session", if live { "attached" } else { "not attached" }.to_owned()),
                ("Presses", if presses.is_empty() { "0".to_owned() } else { presses }),
                ("Note", if note.is_empty() { "—".to_owned() } else { note }),
            ]),
            ui::row([
                ui::button("Press", "press").primary(),
                ui::button("Reset", "reset").danger(),
                ui::badge(if live { "live" } else { "idle" }).tone(if live {
                    ui::Tone::Success
                } else {
                    ui::Tone::Muted
                }),
            ]),
            ui::input("A note for this thread", "Save note", "note"),
            ui::divider(),
            ui::heading("Prompts it understands"),
            ui::table(
                ["Prompt", "What it does"],
                commands.map(|(prompt, effect)| vec![prompt.to_owned(), effect.to_owned()]),
            ),
        ]);
        Ok(tree.to_json())
    }

    fn action(&mut self, action: UiAction) -> Result<String, String> {
        let thread = action.thread.clone().ok_or("this view belongs to a thread")?;
        let presses = format!("ui/presses/{thread}");
        // The web page keeps its drawing in the plugin's store and reads it back.
        if action.view == CANVAS_VIEW {
            let strokes = format!("ui/strokes/{thread}");
            match action.action.as_str() {
                "load" => {}
                "save" => kv::put(&strokes, action.value.unwrap_or_default().as_bytes())?,
                "clear" => kv::remove(&strokes)?,
                other => return Err(format!("unknown action {other:?}")),
            }
            let saved = stored(&strokes)?;
            let strokes: serde_json::Value = serde_json::from_str(&saved).unwrap_or_default();
            return Ok(serde_json::json!({ "strokes": strokes }).to_string());
        }
        let note = format!("ui/note/{thread}");
        match action.action.as_str() {
            "press" => {
                let count = stored(&presses)?.parse::<u64>().unwrap_or(0) + 1;
                kv::put(&presses, count.to_string().as_bytes())?;
            }
            "reset" => {
                kv::remove(&presses)?;
                kv::remove(&note)?;
            }
            "note" => kv::put(&note, action.value.unwrap_or_default().trim().as_bytes())?,
            other => return Err(format!("unknown action {other:?}")),
        }
        self.render(UiRender { view: action.view, thread: action.thread })
    }

    fn render_item(&mut self, item: UiItem) -> Result<String, String> {
        if item.kind != CARD_KIND {
            return Err(format!("unknown item kind {:?}", item.kind));
        }
        let payload: serde_json::Value =
            serde_json::from_str(&item.payload).map_err(|error| error.to_string())?;
        let field = |name: &str| payload[name].as_str().unwrap_or_default().to_owned();
        Ok(ui::stack([
            ui::row([ui::heading(field("title")), ui::badge("card").tone(ui::Tone::Success)]),
            ui::text(field("body")),
        ])
        .to_json())
    }
}

impl Plugin for Echo {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        vec![self]
    }

    /// Example settings, one of each kind a client draws differently.
    fn settings(&self) -> Vec<SettingField> {
        let field = |id: &str, label: &str, kind, default: Option<&str>| SettingField {
            id: id.into(),
            label: label.into(),
            description: None,
            kind,
            default: default.map(str::to_owned),
        };
        vec![
            field("greeting", "Greeting", SettingKind::Text, Some("hello")),
            field("loud", "Loud", SettingKind::Flag, Some("false")),
            field(
                "voice",
                "Voice",
                SettingKind::Choice(vec!["calm".into(), "cheery".into()]),
                None,
            ),
            field("agent", "Agent", SettingKind::Provider, None),
            field("model", "Model", SettingKind::Model("agent".into()), None),
            field("effort", "Effort", SettingKind::Effort(("agent".into(), "model".into())), None),
            field("mode", "Permissions", SettingKind::PermissionMode, None),
        ]
    }

    fn on_settings(&mut self, values: Vec<(String, String)>) {
        self.settings = values;
    }

    fn skills(&self) -> Vec<SkillInfo> {
        vec![SkillInfo {
            name: "echo-demo".into(),
            description: "An example skill, offered only to threads that ask for it.".into(),
            body: "Say everything twice.".into(),
            on_request: true,
        }]
    }

    fn ui(&mut self) -> Option<&mut dyn Ui> {
        Some(self)
    }

    fn on_timer(&mut self, timer: u64) {
        let Some((thread, session)) = self.sessions.iter_mut().find(|(_, session)| {
            matches!(
                &session.turn,
                Some(Turn::Slow { timer: waiting, .. } | Turn::Spin { timer: waiting })
                    if *waiting == timer
            )
        }) else {
            return;
        };
        if let Some(Turn::Spin { timer }) = &mut session.turn {
            match timers::start(0) {
                Ok(next) => *timer = next,
                Err(error) => log::error(format!("cannot restart the spin: {error}")),
            }
            return;
        }
        let Some(Turn::Slow { words, timer }) = &mut session.turn else {
            return;
        };
        let mut out = Vec::new();
        match words.pop_front() {
            Some(word) => {
                out.push(text(format!("{word} ")));
                match timers::start(SLOW_TICK_MS) {
                    Ok(next) => *timer = next,
                    Err(error) => {
                        session.turn = None;
                        out.push(boundary(TurnStatus::Failed, Some(error)));
                    }
                }
            }
            None => {
                session.turn = None;
                out.push(text_close());
                out.push(completed());
            }
        }
        if let Err(error) = deltas::emit(thread, &out) {
            log::error(format!("emit failed: {error}"));
        }
    }

    fn on_process(&mut self, event: ProcessEvent) {
        let (process, out) = match event {
            ProcessEvent::Stdout { process, data } => {
                (process, vec![text(String::from_utf8_lossy(&data))])
            }
            ProcessEvent::Stderr { process, data } => {
                log::warn(format!("process {process}: {}", String::from_utf8_lossy(&data)));
                return;
            }
            ProcessEvent::Exit { process, code } => {
                let end = match code {
                    Some(0) => completed(),
                    Some(code) => boundary(TurnStatus::Failed, Some(format!("exit code {code}"))),
                    None => boundary(TurnStatus::Failed, Some("killed by a signal".into())),
                };
                (process, vec![text_close(), end])
            }
        };
        // Output of a process whose turn was interrupted belongs to nobody.
        let Some(thread) = self.thread_of_process(process) else {
            return;
        };
        if matches!(out.last(), Some(Delta::TurnBoundary(_)))
            && let Some(session) = self.sessions.get_mut(&thread)
        {
            session.turn = None;
        }
        if let Err(error) = deltas::emit(&thread, &out) {
            log::error(format!("emit failed: {error}"));
        }
    }
}

qcc_plugin_sdk::export_plugin!(Echo);
