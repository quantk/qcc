//! Codex provider.
//!
//! One `codex app-server` process per thread, speaking JSON-RPC over stdio. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;
pub mod limits;
pub mod models;
mod probe;
pub mod skills;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{Probes, Sessions};

use bridge::{Bridge, Config};

const PROGRAM: &str = "codex";

#[derive(Default)]
struct Codex {
    sessions: Sessions<Bridge>,
    probes: Probes,
}

impl Provider for Codex {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: "codex".into(),
            name: "Codex".into(),
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
                // Codex reads its MCP servers when a thread starts: qcc's servers a
                // thread keeps off are left out of the new session.
                mcp_live: false,
                compact: true,
                list_skills: true,
                context_usage: false,
                // Codex reads images from their files (`localImage`); the bytes it
                // is given go unused.
                images: true,
                rate_limits: true,
                // `thread/fork` goes through any turn that is over.
                fork: ForkSupport::Any,
            },
        }
    }

    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
        let context = session_context(&start);
        let process = process::spawn(&process::SpawnOptions {
            program: PROGRAM.into(),
            args: vec!["app-server".into()],
            env: start.env,
            cwd: Some(start.cwd.clone()),
        })?;
        let (bridge, handshake) = Bridge::new(Config {
            cwd: start.cwd,
            model: start.model,
            resume: start.resume.clone(),
            fork_at: start.fork_at.clone(),
            mode: start.permission_mode,
            instructions: context,
            effort: start.effort,
            writable_roots: start.extra_dirs,
            mcp_servers: start.mcp_servers,
        });
        self.sessions.insert(&start.thread, process, bridge);

        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        self.sessions.apply(&start.thread, handshake)?;
        if let Some(prompt) = &start.prompt {
            // Held by the bridge until the agent finished its handshake.
            let prompt = message(prompt, &start.prompt_notes, &start.prompt_attachments, true);
            self.sessions.run(&start.thread, |bridge| bridge.prompt_message(&prompt))?;
        }
        // A new thread is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        // A fork is a new thread, named by the agent too.
        Ok(SessionStarted { provider_session: start.resume.filter(|_| start.fork_at.is_none()) })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let message = message(&input.text, &input.notes, &input.attachments, true);
        self.sessions.run(&input.thread, |bridge| bridge.prompt_message(&message))
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let message = message(&input.text, &input.notes, &input.attachments, true);
        self.sessions.run(&input.thread, |bridge| bridge.steer_message(&message))
    }

    fn compact(&mut self, thread: String) -> Result<(), String> {
        self.sessions.run(&thread, Bridge::compact)
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let Ok(bridge) = self.sessions.bridge(&thread) else { return Ok(()) };
        let output = bridge.interrupt();
        self.sessions.apply(&thread, output)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        let Some(mut bridge) = self.sessions.kill(&thread) else { return Ok(()) };
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
        self.probe(models::requests(), models::probe())
    }

    fn skills(&mut self) -> Result<Reply<Vec<AgentSkill>>, String> {
        self.probe(skills::requests(), skills::probe())
    }

    fn rate_limits(&mut self) -> Result<Reply<RateLimits>, String> {
        self.probe(limits::requests(), limits::probe())
    }
}

impl Codex {
    /// Ask a short-lived `codex app-server` one question.
    fn probe<T>(
        &mut self,
        stdin: Vec<String>,
        parser: probe::ListProbe,
    ) -> Result<Reply<T>, String> {
        let options = process::SpawnOptions {
            program: PROGRAM.into(),
            args: vec!["app-server".into()],
            env: Vec::new(),
            cwd: None,
        };
        Ok(Reply::Later(self.probes.start(&options, &stdin, Box::new(parser))?))
    }
}

impl Plugin for Codex {
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

qcc_plugin_sdk::export_plugin!(Codex);
