//! Claude Code provider.
//!
//! One `claude -p` process per thread, speaking stream-json in both directions. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;

use std::collections::HashMap;

use qcc_plugin_sdk::lines::{LineBuffer, Tail};
use qcc_plugin_sdk::prelude::*;

use bridge::{Bridge, Output};

const PROGRAM: &str = "claude";

#[derive(Default)]
struct ClaudeCode {
    sessions: HashMap<String, Session>,
}

struct Session {
    process: u64,
    stdout: LineBuffer,
    stderr: Tail,
    bridge: Bridge,
}

impl Session {
    /// Carry out what the bridge decided: write to the agent, report to the core.
    fn apply(&mut self, thread: &str, output: Output) -> Result<(), String> {
        for line in output.writes {
            process::write_stdin(self.process, format!("{line}\n").as_bytes())?;
        }
        if output.deltas.is_empty() {
            return Ok(());
        }
        deltas::emit(thread, &output.deltas)
    }
}

impl Provider for ClaudeCode {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            capabilities: ProviderCapabilities { resume: true, fork: false, steer: false },
        }
    }

    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
        if let Some(old) = self.sessions.remove(&start.thread) {
            let _ = process::kill(old.process);
        }
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
        if let Some(session) = &start.resume {
            args.extend(["--resume".to_owned(), session.clone()]);
        }
        let process = process::spawn(&process::SpawnOptions {
            program: PROGRAM.into(),
            args,
            env: start.env,
            cwd: Some(start.cwd),
        })?;

        let mut session = Session {
            process,
            stdout: LineBuffer::default(),
            stderr: Tail::new(4096),
            bridge: Bridge::new(start.resume.clone()),
        };
        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        if let Some(prompt) = &start.prompt {
            let output = session.bridge.prompt(prompt);
            session.apply(&start.thread, output)?;
        }
        self.sessions.insert(start.thread, session);
        // A new session is named by the agent's first message; the bridge reports it
        // with `session-identity` when it arrives.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let session = self.sessions.get_mut(&input.thread).ok_or("the session is not running")?;
        let output = session.bridge.prompt(&input.text);
        session.apply(&input.thread, output)
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(&thread) else {
            return Ok(());
        };
        let output = session.bridge.interrupt();
        session.apply(&thread, output)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        let Some(mut session) = self.sessions.remove(&thread) else {
            return Ok(());
        };
        let _ = process::kill(session.process);
        deltas::emit(&thread, &session.bridge.stopped())
    }

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let session = self.sessions.get_mut(&answer.thread).ok_or("the session is not running")?;
        let output = session.bridge.resolve(&answer.request_id, &answer.resolution)?;
        session.apply(&answer.thread, output)
    }

    fn models(&mut self) -> Result<Vec<ModelInfo>, String> {
        // Aliases the CLI resolves to the current model of each family.
        Ok(["opus", "sonnet", "haiku"]
            .map(|id| ModelInfo { id: id.into(), name: id.into() })
            .into())
    }
}

impl Plugin for ClaudeCode {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        vec![self]
    }

    fn on_process(&mut self, event: ProcessEvent) {
        let process = match &event {
            ProcessEvent::Stdout { process, .. }
            | ProcessEvent::Stderr { process, .. }
            | ProcessEvent::Exit { process, .. } => *process,
        };
        // A process whose session was stopped has nobody to report to.
        let Some(thread) = self
            .sessions
            .iter()
            .find(|(_, session)| session.process == process)
            .map(|(thread, _)| thread.clone())
        else {
            return;
        };
        let result = match event {
            ProcessEvent::Stdout { data, .. } => {
                let session = self.sessions.get_mut(&thread).expect("found above");
                let mut result = Ok(());
                for line in session.stdout.push(&data) {
                    let output = session.bridge.on_line(&line);
                    result = result.and(session.apply(&thread, output));
                }
                result
            }
            ProcessEvent::Stderr { data, .. } => {
                self.sessions.get_mut(&thread).expect("found above").stderr.push(&data);
                Ok(())
            }
            ProcessEvent::Exit { code, .. } => {
                let mut session = self.sessions.remove(&thread).expect("found above");
                let ending = session.bridge.exited(code, session.stderr.last_line());
                deltas::emit(&thread, &ending)
            }
        };
        if let Err(error) = result {
            log::error(format!("thread {thread}: {error}"));
        }
    }
}

qcc_plugin_sdk::export_plugin!(ClaudeCode);
