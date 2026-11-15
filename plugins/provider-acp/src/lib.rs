//! Agent Client Protocol provider.
//!
//! One bridge for every agent that speaks ACP over stdio. Each agent in [`AGENTS`] is
//! contributed as its own provider; they differ only in the command that starts them.
//! The dialect lives in [`bridge`], which is pure.

pub mod bridge;

use std::collections::HashMap;

use qcc_plugin_sdk::lines::{LineBuffer, Tail};
use qcc_plugin_sdk::prelude::*;

use bridge::{Bridge, Output};

/// An agent this plugin can run.
struct Agent {
    id: &'static str,
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
}

/// Adding an ACP agent is one line here plus its executable in `plugin.toml`.
const AGENTS: &[Agent] =
    &[Agent { id: "opencode", name: "opencode", program: "opencode", args: &["acp"] }];

#[derive(Default)]
struct Acp {
    providers: Vec<AcpProvider>,
}

struct AcpProvider {
    agent: &'static Agent,
    sessions: HashMap<String, Session>,
}

struct Session {
    process: u64,
    stdout: LineBuffer,
    stderr: Tail,
    bridge: Bridge,
}

impl Session {
    /// Carry out what the bridge decided. Returns whether the session is over.
    fn apply(&mut self, thread: &str, output: Output) -> Result<bool, String> {
        for line in output.writes {
            process::write_stdin(self.process, format!("{line}\n").as_bytes())?;
        }
        if !output.deltas.is_empty() {
            deltas::emit(thread, &output.deltas)?;
        }
        if output.fatal {
            let _ = process::kill(self.process);
        }
        Ok(output.fatal)
    }
}

impl AcpProvider {
    fn apply(&mut self, thread: &str, output: Output) -> Result<(), String> {
        let session = self.sessions.get_mut(thread).ok_or("the session is not running")?;
        if session.apply(thread, output)? {
            self.sessions.remove(thread);
        }
        Ok(())
    }

    fn on_process(&mut self, thread: String, event: ProcessEvent) -> Result<(), String> {
        match event {
            ProcessEvent::Stdout { data, .. } => {
                let Some(session) = self.sessions.get_mut(&thread) else { return Ok(()) };
                for line in session.stdout.push(&data) {
                    // A fatal line removes the session; the rest of the chunk is moot.
                    let Some(session) = self.sessions.get_mut(&thread) else { break };
                    let output = session.bridge.on_line(&line);
                    self.apply(&thread, output)?;
                }
                Ok(())
            }
            ProcessEvent::Stderr { data, .. } => {
                if let Some(session) = self.sessions.get_mut(&thread) {
                    session.stderr.push(&data);
                }
                Ok(())
            }
            ProcessEvent::Exit { code, .. } => {
                let Some(mut session) = self.sessions.remove(&thread) else { return Ok(()) };
                let name = self.agent.name;
                deltas::emit(
                    &thread,
                    &session.bridge.exited(name, code, session.stderr.last_line()),
                )
            }
        }
    }
}

impl Provider for AcpProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.agent.id.into(),
            name: self.agent.name.into(),
            capabilities: ProviderCapabilities {
                resume: true,
                fork: false,
                // The protocol has no way to add input to a running prompt.
                steer: false,
                // ACP has no automatic reviewer for `auto` to hand approvals to; the
                // other modes are enforced by the bridge answering permission requests.
                permission_modes: vec![
                    PermissionMode::Ask,
                    PermissionMode::AcceptEdits,
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
            program: self.agent.program.into(),
            args: self.agent.args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: start.env,
            cwd: Some(start.cwd.clone()),
        })?;
        let (bridge, handshake) =
            Bridge::new(start.cwd, start.model, start.resume.clone(), start.permission_mode);
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
        // A new session is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let session = self.sessions.get_mut(&input.thread).ok_or("the session is not running")?;
        let output = session.bridge.prompt(&input.text);
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

impl Plugin for Acp {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        if self.providers.is_empty() {
            self.providers = AGENTS
                .iter()
                .map(|agent| AcpProvider { agent, sessions: HashMap::new() })
                .collect();
        }
        self.providers.iter_mut().map(|provider| provider as &mut dyn Provider).collect()
    }

    fn on_process(&mut self, event: ProcessEvent) {
        let process = match &event {
            ProcessEvent::Stdout { process, .. }
            | ProcessEvent::Stderr { process, .. }
            | ProcessEvent::Exit { process, .. } => *process,
        };
        // A process whose session was stopped has nobody to report to.
        for provider in &mut self.providers {
            let thread = provider
                .sessions
                .iter()
                .find(|(_, session)| session.process == process)
                .map(|(thread, _)| thread.clone());
            if let Some(thread) = thread {
                if let Err(error) = provider.on_process(thread.clone(), event) {
                    log::error(format!("thread {thread}: {error}"));
                }
                return;
            }
        }
    }
}

qcc_plugin_sdk::export_plugin!(Acp);
