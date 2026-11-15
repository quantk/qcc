//! Claude Code provider.
//!
//! One `claude -p` process per thread, speaking stream-json in both directions. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;
pub mod models;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{Probes, Sessions};

use bridge::Bridge;

const PROGRAM: &str = "claude";

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
                fork: false,
                steer: true,
                permission_modes: vec![
                    PermissionMode::Ask,
                    PermissionMode::AcceptEdits,
                    PermissionMode::Auto,
                    PermissionMode::Full,
                ],
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
        if let Some(mode) = start.permission_mode {
            // Without the flag the user's own Claude Code settings decide.
            let mode = match mode {
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

        self.sessions.insert(&start.thread, process, Bridge::new(start.resume.clone()));
        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        if let Some(prompt) = &start.prompt {
            let output = self.sessions.bridge(&start.thread)?.prompt(prompt);
            self.sessions.apply(&start.thread, output)?;
        }
        // A new session is named by the agent's first message; the bridge reports it
        // with `session-identity` when it arrives.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let output = self.sessions.bridge(&input.thread)?.prompt(&input.text);
        self.sessions.apply(&input.thread, output)
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let output = self.sessions.bridge(&input.thread)?.steer(&input.text);
        self.sessions.apply(&input.thread, output)
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

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let output = self
            .sessions
            .bridge(&answer.thread)?
            .resolve(&answer.request_id, &answer.resolution)?;
        self.sessions.apply(&answer.thread, output)
    }

    fn models(&mut self) -> Result<Reply<Vec<ModelInfo>>, String> {
        let options = process::SpawnOptions {
            program: PROGRAM.into(),
            args: models::ARGS.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Vec::new(),
            cwd: None,
        };
        let token =
            self.probes.start(&options, &[models::request()], Box::new(models::ModelsProbe))?;
        Ok(Reply::Later(token))
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
        self.probes.on_timer(timer);
    }
}

qcc_plugin_sdk::export_plugin!(ClaudeCode);
