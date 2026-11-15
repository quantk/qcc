//! Agent Client Protocol provider.
//!
//! One bridge for every agent that speaks ACP over stdio. Each agent in [`AGENTS`] is
//! contributed as its own provider; they differ only in the command that starts them.
//! The dialect lives in [`bridge`], which is pure.

pub mod bridge;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{LineBridge, Output, Sessions};

use bridge::Bridge;

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
    sessions: Sessions<Running>,
}

/// A bridge and the agent it talks to, which names it when the process dies.
struct Running {
    agent: &'static str,
    bridge: Bridge,
}

impl LineBridge for Running {
    fn on_line(&mut self, line: &str) -> Output {
        self.bridge.on_line(line)
    }

    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta> {
        self.bridge.exited(self.agent, code, stderr)
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
        let process = process::spawn(&process::SpawnOptions {
            program: self.agent.program.into(),
            args: self.agent.args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: start.env,
            cwd: Some(start.cwd.clone()),
        })?;
        let (mut bridge, handshake) =
            Bridge::new(start.cwd, start.model, start.resume.clone(), start.permission_mode);
        bridge.set_context(skills_prompt(&start.skills));
        self.sessions.insert(&start.thread, process, Running { agent: self.agent.name, bridge });

        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        self.sessions.apply(&start.thread, handshake)?;
        if let Some(prompt) = &start.prompt {
            // Held by the bridge until the agent finished its handshake.
            let output = self.sessions.bridge(&start.thread)?.bridge.prompt(prompt);
            self.sessions.apply(&start.thread, output)?;
        }
        // A new session is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let output = self.sessions.bridge(&input.thread)?.bridge.prompt(&input.text);
        self.sessions.apply(&input.thread, output)
    }

    fn interrupt(&mut self, thread: String) -> Result<(), String> {
        let Ok(running) = self.sessions.bridge(&thread) else { return Ok(()) };
        let output = running.bridge.interrupt();
        self.sessions.apply(&thread, output)
    }

    fn stop(&mut self, thread: String) -> Result<(), String> {
        let Some(mut running) = self.sessions.kill(&thread) else { return Ok(()) };
        deltas::emit(&thread, &running.bridge.stopped())
    }

    fn resolve_interaction(&mut self, answer: InteractionAnswer) -> Result<(), String> {
        let running = self.sessions.bridge(&answer.thread)?;
        let output = running.bridge.resolve(&answer.request_id, &answer.resolution)?;
        self.sessions.apply(&answer.thread, output)
    }
}

impl Plugin for Acp {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        if self.providers.is_empty() {
            self.providers = AGENTS
                .iter()
                .map(|agent| AcpProvider { agent, sessions: Sessions::default() })
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
        if let Some(provider) = self.providers.iter_mut().find(|p| p.sessions.owns(process)) {
            provider.sessions.on_process(event);
        }
    }
}

qcc_plugin_sdk::export_plugin!(Acp);
