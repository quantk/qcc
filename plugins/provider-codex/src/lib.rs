//! Codex provider.
//!
//! One `codex app-server` process per thread, speaking JSON-RPC over stdio. The
//! dialect lives in [`bridge`], which is pure; this file only moves bytes between the
//! process and the bridge.

pub mod bridge;
pub mod models;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{Probes, Sessions};

use bridge::Bridge;

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
                fork: false,
                steer: true,
                permission_modes: vec![
                    PermissionMode::Ask,
                    PermissionMode::AcceptEdits,
                    PermissionMode::Auto,
                    PermissionMode::Full,
                ],
                // Codex reads its MCP servers when a thread starts: qcc's servers a
                // thread keeps off are left out of the new session.
                mcp_toggle: false,
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
        let (mut bridge, handshake) =
            Bridge::new(start.cwd, start.model, start.resume.clone(), start.permission_mode);
        bridge.set_instructions(context);
        bridge.set_effort(start.effort);
        bridge.set_writable_roots(start.extra_dirs);
        bridge.set_mcp_servers(start.mcp_servers);
        self.sessions.insert(&start.thread, process, bridge);

        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        self.sessions.apply(&start.thread, handshake)?;
        if let Some(prompt) = &start.prompt {
            // Held by the bridge until the agent finished its handshake.
            let output = self
                .sessions
                .bridge(&start.thread)?
                .prompt(&with_notes(&start.prompt_notes, prompt));
            self.sessions.apply(&start.thread, output)?;
        }
        // A new thread is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let output =
            self.sessions.bridge(&input.thread)?.prompt(&with_notes(&input.notes, &input.text));
        self.sessions.apply(&input.thread, output)
    }

    fn steer(&mut self, input: ThreadInput) -> Result<(), String> {
        let output =
            self.sessions.bridge(&input.thread)?.steer(&with_notes(&input.notes, &input.text));
        self.sessions.apply(&input.thread, output)
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
        let options = process::SpawnOptions {
            program: PROGRAM.into(),
            args: vec!["app-server".into()],
            env: Vec::new(),
            cwd: None,
        };
        let token =
            self.probes.start(&options, &models::requests(), Box::new(models::ModelsProbe))?;
        Ok(Reply::Later(token))
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
