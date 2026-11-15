//! Codex provider.
//!
//! One `codex app-server` process per thread, speaking JSON-RPC over stdio. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;

use std::collections::HashMap;

use qcc_plugin_sdk::lines::{LineBuffer, Tail};
use qcc_plugin_sdk::prelude::*;

use bridge::{Bridge, Output};

const PROGRAM: &str = "codex";

#[derive(Default)]
struct Codex {
    sessions: HashMap<String, Session>,
}

struct Session {
    process: u64,
    stdout: LineBuffer,
    stderr: Tail,
    bridge: Bridge,
}

impl Codex {
    /// Carry out what the bridge decided: write to the agent, report to the core,
    /// and drop the session if it cannot go on.
    fn apply(&mut self, thread: &str, output: Output) -> Result<(), String> {
        let session = self.sessions.get_mut(thread).ok_or("the session is not running")?;
        for line in output.writes {
            process::write_stdin(session.process, format!("{line}\n").as_bytes())?;
        }
        if !output.deltas.is_empty() {
            deltas::emit(thread, &output.deltas)?;
        }
        if output.fatal {
            let _ = process::kill(session.process);
            self.sessions.remove(thread);
        }
        Ok(())
    }

    fn process_event(&mut self, thread: &str, event: ProcessEvent) -> Result<(), String> {
        match event {
            ProcessEvent::Stdout { data, .. } => {
                let Some(session) = self.sessions.get_mut(thread) else { return Ok(()) };
                for line in session.stdout.push(&data) {
                    // A fatal line removes the session; the rest of the chunk is moot.
                    let Some(session) = self.sessions.get_mut(thread) else { break };
                    let output = session.bridge.on_line(&line);
                    self.apply(thread, output)?;
                }
                Ok(())
            }
            ProcessEvent::Stderr { data, .. } => {
                if let Some(session) = self.sessions.get_mut(thread) {
                    session.stderr.push(&data);
                }
                Ok(())
            }
            ProcessEvent::Exit { code, .. } => {
                let Some(mut session) = self.sessions.remove(thread) else { return Ok(()) };
                deltas::emit(thread, &session.bridge.exited(code, session.stderr.last_line()))
            }
        }
    }
}

impl Provider for Codex {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "codex".into(),
            name: "Codex".into(),
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
        if let Some(old) = self.sessions.remove(&start.thread) {
            let _ = process::kill(old.process);
        }
        let process = process::spawn(&process::SpawnOptions {
            program: PROGRAM.into(),
            args: vec!["app-server".into()],
            env: start.env,
            cwd: Some(start.cwd.clone()),
        })?;
        let (mut bridge, handshake) =
            Bridge::new(start.cwd, start.model, start.resume.clone(), start.permission_mode);
        bridge.set_instructions(skills_prompt(&start.skills));
        let session =
            Session { process, stdout: LineBuffer::default(), stderr: Tail::new(4096), bridge };
        self.sessions.insert(start.thread.clone(), session);

        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        self.apply(&start.thread, handshake)?;
        if let Some(prompt) = &start.prompt {
            // Held by the bridge until the agent finished its handshake.
            let output = self.sessions.get_mut(&start.thread).map(|s| s.bridge.prompt(prompt));
            self.apply(&start.thread, output.unwrap_or_default())?;
        }
        // A new thread is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let session = self.sessions.get_mut(&input.thread).ok_or("the session is not running")?;
        let output = session.bridge.prompt(&input.text);
        self.apply(&input.thread, output)
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let session = self.sessions.get_mut(&input.thread).ok_or("the session is not running")?;
        let output = session.bridge.steer(&input.text);
        self.apply(&input.thread, output)
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let Some(session) = self.sessions.get_mut(&thread) else { return Ok(()) };
        let output = session.bridge.interrupt();
        self.apply(&thread, output)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        let Some(mut session) = self.sessions.remove(&thread) else { return Ok(()) };
        let _ = process::kill(session.process);
        deltas::emit(&thread, &session.bridge.stopped())
    }

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let session = self.sessions.get_mut(&answer.thread).ok_or("the session is not running")?;
        let output = session.bridge.resolve(&answer.request_id, &answer.resolution)?;
        self.apply(&answer.thread, output)
    }
}

impl Plugin for Codex {
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
        if let Err(error) = self.process_event(&thread, event) {
            log::error(format!("thread {thread}: {error}"));
        }
    }
}

qcc_plugin_sdk::export_plugin!(Codex);
