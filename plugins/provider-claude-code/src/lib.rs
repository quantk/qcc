//! Claude Code provider.
//!
//! One `claude -p` process per thread, speaking stream-json in both directions. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;
pub mod limits;
pub mod models;
mod probe;
pub mod skills;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{Probes, Sessions};

use bridge::Bridge;

const PROGRAM: &str = "claude";

/// Commands a read-only session may run: they only look.
const READ_ONLY_COMMANDS: [&str; 11] = [
    "git diff",
    "git log",
    "git show",
    "git status",
    "git blame",
    "git ls-files",
    "qcc thread diff",
    "qcc thread changes",
    "qcc thread show",
    "qcc comment list",
    "ls",
];

/// How a read-only session is started. `dontAsk` refuses whatever is not allowed
/// here without asking; only the tools that read are there at all, and Bash only
/// for the commands that look. `--restricted` keeps the user's settings files out,
/// so allow rules of their own cannot loosen it.
fn read_only_args() -> Vec<String> {
    let mut args: Vec<String> =
        ["--permission-mode", "dontAsk", "--restricted", "--tools", "Read,Grep,Glob,Bash"]
            .map(String::from)
            .into();
    args.push("--allowedTools".into());
    args.extend(READ_ONLY_COMMANDS.iter().map(|command| format!("Bash({command}:*)")));
    args
}

#[derive(Default)]
struct ClaudeCode {
    sessions: Sessions<Bridge>,
    probes: Probes,
}

impl Provider for ClaudeCode {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            capabilities: ProviderCapabilities {
                resume: true,
                steer: true,
                permission_modes: vec![
                    PermissionMode::ReadOnly,
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
                fork: ForkSupport::None,
            },
        }
    }

    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
        let mut args: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            // Permission prompts come to us as control requests instead of being denied.
            "--permission-prompt-tool",
            "stdio",
        ]
        .map(String::from)
        .into();
        if let Some(model) = &start.model {
            args.extend(["--model".to_owned(), model.clone()]);
        }
        if let Some(effort) = &start.effort {
            args.extend(["--effort".to_owned(), effort.clone()]);
        }
        if start.permission_mode == Some(PermissionMode::ReadOnly) {
            args.extend(read_only_args());
        } else if let Some(mode) = start.permission_mode {
            // Without the flag the user's own Claude Code settings decide.
            let mode = match mode {
                PermissionMode::ReadOnly => unreachable!("handled above"),
                PermissionMode::Ask => "manual",
                PermissionMode::AcceptEdits => "acceptEdits",
                PermissionMode::Auto => "auto",
                PermissionMode::Full => "bypassPermissions",
            };
            args.extend(["--permission-mode".to_owned(), mode.to_owned()]);
        }
        if let Some(session) = &start.resume {
            args.extend(["--resume".to_owned(), session.clone()]);
        }
        // The project's other folders are working directories too.
        for dir in &start.extra_dirs {
            args.extend(["--add-dir".to_owned(), dir.clone()]);
        }
        // Added to the servers of the user's Claude Code configuration. The flag takes
        // several values, so the config goes in the same argument.
        if let Some(config) = bridge::mcp_config(&start.mcp_servers) {
            args.push(format!("--mcp-config={config}"));
        }
        if let Some(skills) = skills_prompt(&start.skills) {
            args.extend(["--append-system-prompt".to_owned(), skills]);
        }
        let process = process::spawn(&process::SpawnOptions {
            program: PROGRAM.into(),
            args,
            env: start.env,
            cwd: Some(start.cwd),
        })?;

        let mut bridge = Bridge::new(start.resume.clone());
        if start.permission_mode == Some(PermissionMode::ReadOnly) {
            bridge = bridge.read_only();
        }
        self.sessions.insert(&start.thread, process, bridge);
        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        if let Some(prompt) = &start.prompt {
            let prompt = message(prompt, &start.prompt_notes, &start.prompt_attachments, true);
            self.sessions.run(&start.thread, |bridge| bridge.prompt_message(&prompt))?;
        }
        // After the prompt, which must not wait on it; the agent answers either way.
        // Only a status: an agent already gone is reported by its exit, not here.
        if let Ok(bridge) = self.sessions.bridge(&start.thread) {
            let mut query = bridge.query_mcp();
            // What the context holds before a word is said: prompts, tools, memory.
            query.writes.extend(bridge.query_context().writes);
            // The account's limits, before the first turn reports them.
            query.writes.extend(bridge.query_limits().writes);
            if let Err(error) = self.sessions.apply(&start.thread, query) {
                log::debug(format!("thread {}: cannot ask for MCP status: {error}", start.thread));
            }
        }
        // A new session is named by the agent's first message; the bridge reports it
        // with `session-identity` when it arrives.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let message = message(&input.text, &input.notes, &input.attachments, true);
        self.sessions.run(&input.thread, |bridge| bridge.prompt_message(&message))
    }

    fn compact(&mut self, thread: String) -> Result<(), String> {
        self.sessions.run(&thread, Bridge::compact)
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let message = message(&input.text, &input.notes, &input.attachments, true);
        self.sessions.run(&input.thread, |bridge| bridge.steer_message(&message))
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let Ok(bridge) = self.sessions.bridge(&thread) else {
            return Ok(());
        };
        let output = bridge.interrupt();
        self.sessions.apply(&thread, output)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        let Some(mut bridge) = self.sessions.kill(&thread) else {
            return Ok(());
        };
        deltas::emit(&thread, &bridge.stopped())
    }

    fn set_mcp_servers(&mut self, update: McpServersUpdate) -> Result<(), String> {
        self.sessions.run(&update.thread, |bridge| bridge.set_mcp_servers(&update.servers))
    }

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let output = self
            .sessions
            .bridge(&answer.thread)?
            .resolve(&answer.request_id, &answer.resolution)?;
        self.sessions.apply(&answer.thread, output)
    }

    fn models(&mut self) -> Result<Reply<Vec<ModelInfo>>, String> {
        self.probe(&[models::request()], models::probe())
    }

    fn skills(&mut self) -> Result<Reply<Vec<AgentSkill>>, String> {
        self.probe(&skills::requests(), skills::probe())
    }

    fn rate_limits(&mut self) -> Result<Reply<RateLimits>, String> {
        self.probe(&[limits::probe_request()], limits::probe())
    }
}

impl ClaudeCode {
    /// Ask a short-lived `claude` process one question.
    fn probe<T>(
        &mut self,
        stdin: &[String],
        parser: probe::ControlProbe,
    ) -> Result<Reply<T>, String> {
        let options = process::SpawnOptions {
            program: PROGRAM.into(),
            args: probe::ARGS.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Vec::new(),
            cwd: None,
        };
        Ok(Reply::Later(self.probes.start(&options, stdin, Box::new(parser))?))
    }
}

impl Plugin for ClaudeCode {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        vec![self]
    }

    fn on_process(&mut self, event: ProcessEvent) {
        if let Some(event) = self.probes.on_process(event) {
            self.sessions.on_process(event);
        }
    }

    fn on_timer(&mut self, timer: u64) {
        if !self.probes.on_timer(timer) {
            self.sessions.on_timer(timer);
        }
    }
}

qcc_plugin_sdk::export_plugin!(ClaudeCode);
