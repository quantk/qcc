//! Agent Client Protocol provider.
//!
//! One bridge for every agent that speaks ACP over stdio. Each agent in [`AGENTS`] is
//! contributed as its own provider; they differ only in the command that starts them.
//! The dialect lives in [`bridge`], which is pure.

pub mod bridge;
pub mod models;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::stdio::{Probes, Sessions};

use bridge::{Bridge, Config, Prompt};

/// An agent this plugin can run.
struct Agent {
    id: &'static str,
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    /// The arguments that make it print its models, one id per line.
    models_args: Option<&'static [&'static str]>,
}

/// Adding an ACP agent is one line here plus its executable in `plugin.toml`.
const AGENTS: &[Agent] = &[Agent {
    id: "opencode",
    name: "opencode",
    program: "opencode",
    args: &["acp"],
    models_args: Some(&["models"]),
}];

#[derive(Default)]
struct Acp {
    providers: Vec<AcpProvider>,
}

struct AcpProvider {
    agent: &'static Agent,
    sessions: Sessions<Bridge>,
    probes: Probes,
}

impl Provider for AcpProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            id: self.agent.id.into(),
            name: self.agent.name.into(),
            capabilities: ProviderCapabilities {
                resume: true,
                // The protocol has no way to add input to a running prompt.
                steer: false,
                // ACP has no automatic reviewer for `auto` to hand approvals to; the
                // other modes are enforced by the bridge answering permission requests.
                permission_modes: vec![
                    PermissionMode::Ask,
                    PermissionMode::AcceptEdits,
                    PermissionMode::Full,
                ],
                // ACP has no way to turn a server off; qcc's are left out instead.
                mcp_live: false,
                // ACP has no way to ask for it.
                compact: false,
                // The protocol's command list mixes skills with commands, and
                // `opencode debug skill` loses output written into a pipe.
                list_skills: false,
                context_usage: false,
                // As images when the agent says it takes them, by path otherwise.
                images: true,
            },
        }
    }

    fn start(&mut self, start: SessionStart) -> Result<SessionStarted, String> {
        let context = session_context(&start);
        let process = process::spawn(&process::SpawnOptions {
            program: self.agent.program.into(),
            args: self.agent.args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: start.env,
            cwd: Some(start.cwd.clone()),
        })?;
        let (bridge, handshake) = Bridge::new(Config {
            agent: self.agent.name,
            cwd: start.cwd,
            model: start.model,
            resume: start.resume.clone(),
            mode: start.permission_mode,
            // ACP has no extra directories: the agent is told of them instead.
            context,
            mcp_servers: start.mcp_servers,
        });
        self.sessions.insert(&start.thread, process, bridge);

        deltas::emit(&start.thread, &[Delta::SessionReset])?;
        self.sessions.apply(&start.thread, handshake)?;
        if let Some(prompt) = &start.prompt {
            // Held by the bridge until the agent finished its handshake.
            let prompt = Prompt {
                text: prompt.clone(),
                notes: start.prompt_notes,
                attachments: start.prompt_attachments,
            };
            self.sessions.run(&start.thread, |bridge| bridge.prompt_with(prompt))?;
        }
        // A new session is named by the agent after the handshake; the bridge reports
        // it with `session-identity`.
        Ok(SessionStarted { provider_session: start.resume })
    }

    fn input(&mut self, input: ThreadInput) -> Result<(), String> {
        let prompt =
            Prompt { text: input.text, notes: input.notes, attachments: input.attachments };
        self.sessions.run(&input.thread, |bridge| bridge.prompt_with(prompt))
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
        let bridge = self.sessions.bridge(&answer.thread)?;
        let output = bridge.resolve(&answer.request_id, &answer.resolution)?;
        self.sessions.apply(&answer.thread, output)
    }

    fn models(&mut self) -> Result<Reply<Vec<ModelInfo>>, String> {
        let Some(args) = self.agent.models_args else { return Ok(Reply::Now(Vec::new())) };
        let options = process::SpawnOptions {
            program: self.agent.program.into(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            env: Vec::new(),
            cwd: None,
        };
        let probe = Box::new(models::ModelsProbe::new(self.agent.name));
        Ok(Reply::Later(self.probes.start(&options, &[], probe)?))
    }
}

impl Plugin for Acp {
    fn providers(&mut self) -> Vec<&mut dyn Provider> {
        if self.providers.is_empty() {
            self.providers = AGENTS
                .iter()
                .map(|agent| AcpProvider {
                    agent,
                    sessions: Sessions::default(),
                    probes: Probes::default(),
                })
                .collect();
        }
        self.providers.iter_mut().map(|provider| provider as &mut dyn Provider).collect()
    }

    fn on_process(&mut self, mut event: ProcessEvent) {
        for provider in &mut self.providers {
            match provider.probes.on_process(event) {
                Some(unclaimed) => event = unclaimed,
                None => return,
            }
        }
        let process = event.process();
        if let Some(provider) = self.providers.iter_mut().find(|p| p.sessions.owns(process)) {
            provider.sessions.on_process(event);
        }
    }

    fn on_timer(&mut self, timer: u64) {
        for provider in &mut self.providers {
            if provider.probes.on_timer(timer) {
                return;
            }
        }
    }
}

qcc_plugin_sdk::export_plugin!(Acp);
