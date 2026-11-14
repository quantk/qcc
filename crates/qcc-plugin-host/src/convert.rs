//! Translation between the WIT types and the domain. The only place that knows both.

use qcc_domain::{
    Delta, EnvironmentInfo, InteractionOption, InteractionRequest, InteractionResolution, ItemKey,
    ItemShape, ItemStatus, ModelInfo, OptionKind, ProviderCapabilities, ProviderInfo, TextChannel,
    TokenUsage, TurnStatus, UsageSnapshot,
};

use crate::api::{
    Contribution, EnvironmentRequest, Outcome, PluginRequest, ProviderRequest, SessionStart,
};
use crate::bindings::wit;

fn json(text: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))
}

fn key(key: wit::ItemKey) -> ItemKey {
    match key {
        wit::ItemKey::Provider(id) => ItemKey::Provider(id),
        wit::ItemKey::Anonymous(name) => ItemKey::Stream(name),
    }
}

fn channel(channel: wit::TextChannel) -> TextChannel {
    match channel {
        wit::TextChannel::AgentMessage => TextChannel::AgentMessage,
        wit::TextChannel::Reasoning => TextChannel::Reasoning,
        wit::TextChannel::Plan => TextChannel::Plan,
    }
}

fn item_status(status: wit::ItemStatus) -> ItemStatus {
    match status {
        wit::ItemStatus::Pending => ItemStatus::Pending,
        wit::ItemStatus::Completed => ItemStatus::Completed,
        wit::ItemStatus::Failed => ItemStatus::Failed,
        wit::ItemStatus::Declined => ItemStatus::Declined,
        wit::ItemStatus::Interrupted => ItemStatus::Interrupted,
    }
}

fn turn_status(status: wit::TurnStatus) -> TurnStatus {
    match status {
        wit::TurnStatus::Completed => TurnStatus::Completed,
        wit::TurnStatus::Failed => TurnStatus::Failed,
        wit::TurnStatus::Interrupted => TurnStatus::Interrupted,
    }
}

fn shape(shape: wit::ItemShape) -> Result<ItemShape, String> {
    Ok(match shape {
        wit::ItemShape::AgentMessage(text) => ItemShape::AgentMessage { text },
        wit::ItemShape::Reasoning(text) => ItemShape::Reasoning { text },
        wit::ItemShape::Plan(text) => ItemShape::Plan { text },
        wit::ItemShape::Command(c) => ItemShape::Command {
            command: c.command,
            cwd: c.cwd,
            output: c.output,
            exit_code: c.exit_code,
        },
        wit::ItemShape::FileChange(c) => ItemShape::FileChange {
            path: c.path,
            diff: c.diff,
        },
        wit::ItemShape::FileRead(path) => ItemShape::FileRead { path },
        wit::ItemShape::Search(query) => ItemShape::Search { query },
        wit::ItemShape::Tool(t) => ItemShape::Tool {
            name: t.name,
            input: json(&t.input)?,
            output: t.output,
        },
        wit::ItemShape::Extension(e) => ItemShape::Extension {
            kind: e.kind,
            payload: json(&e.payload)?,
        },
    })
}

fn token_usage(usage: wit::TokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
    }
}

fn options(options: Vec<wit::InteractionOption>) -> Vec<InteractionOption> {
    options
        .into_iter()
        .map(|option| InteractionOption {
            id: option.id,
            label: option.label,
            kind: match option.kind {
                wit::OptionKind::Allow => OptionKind::Allow,
                wit::OptionKind::Deny => OptionKind::Deny,
                wit::OptionKind::Other => OptionKind::Other,
            },
        })
        .collect()
}

fn interaction_request(request: wit::InteractionRequest) -> InteractionRequest {
    match request {
        wit::InteractionRequest::Approval(a) => InteractionRequest::Approval {
            title: a.title,
            detail: a.detail,
            options: options(a.options),
        },
        wit::InteractionRequest::Question(q) => InteractionRequest::Question {
            question: q.question,
            options: options(q.options),
            allow_text: q.allow_text,
        },
    }
}

/// A delta as the plugin sent it, in domain terms. Fails on malformed embedded JSON:
/// what a delta assembles into is persisted, so the core stays strict.
pub(crate) fn delta(delta: wit::Delta) -> Result<Delta, String> {
    Ok(match delta {
        wit::Delta::TurnOpen => Delta::TurnOpen,
        wit::Delta::TurnBoundary(b) => Delta::TurnBoundary {
            status: turn_status(b.status),
            error: b.error,
            claim_if_idle: b.claim_if_idle,
        },
        wit::Delta::ItemOpen(o) => Delta::ItemOpen {
            key: key(o.key),
            shape: shape(o.shape)?,
        },
        wit::Delta::ItemClose(c) => Delta::ItemClose {
            key: key(c.key),
            shape: shape(c.shape)?,
            status: item_status(c.status),
        },
        wit::Delta::ItemTextDelta(d) => Delta::ItemTextDelta {
            key: key(d.key),
            channel: channel(d.channel),
            text: d.text,
        },
        wit::Delta::ItemTextClose(c) => Delta::ItemTextClose {
            key: key(c.key),
            channel: channel(c.channel),
            text: c.text,
        },
        wit::Delta::Usage(u) => Delta::Usage {
            usage: UsageSnapshot {
                total: token_usage(u.total),
                last: token_usage(u.last),
                model_context_window: u.model_context_window,
            },
        },
        wit::Delta::InteractionRequest(ask) => Delta::InteractionRequest {
            request_id: ask.request_id,
            item: ask.item.map(key),
            request: interaction_request(ask.request),
        },
        wit::Delta::SessionReset => Delta::SessionReset,
        wit::Delta::SessionEnded(reason) => Delta::SessionEnded { reason },
        wit::Delta::ProviderError(message) => Delta::ProviderError { message },
        wit::Delta::ProviderWarning(message) => Delta::ProviderWarning { message },
        wit::Delta::Unhandled(payload) => Delta::Unhandled {
            payload: json(&payload)?,
        },
    })
}

pub(crate) fn contribution(contribution: wit::Contribution) -> Contribution {
    match contribution {
        wit::Contribution::Provider(p) => Contribution::Provider(ProviderInfo {
            id: p.id.into(),
            name: p.name,
            capabilities: ProviderCapabilities {
                resume: p.capabilities.resume,
                fork: p.capabilities.fork,
                steer: p.capabilities.steer,
            },
        }),
        wit::Contribution::Environment(e) => Contribution::Environment(EnvironmentInfo {
            id: e.id,
            name: e.name,
        }),
    }
}

pub(crate) fn outcome(outcome: wit::Outcome) -> Outcome {
    match outcome {
        wit::Outcome::Done => Outcome::Done,
        wit::Outcome::SessionStarted(s) => Outcome::SessionStarted {
            provider_session: s.provider_session,
        },
        wit::Outcome::Models(models) => Outcome::Models(
            models
                .into_iter()
                .map(|m| ModelInfo {
                    id: m.id,
                    name: m.name,
                })
                .collect(),
        ),
        wit::Outcome::EnvironmentReady(cwd) => Outcome::EnvironmentReady { cwd },
    }
}

fn session_start(start: SessionStart) -> wit::SessionStart {
    wit::SessionStart {
        thread: start.thread.0,
        cwd: start.cwd,
        model: start.model,
        resume: start.resume,
        prompt: start.prompt,
        env: start.env,
    }
}

fn resolution(resolution: InteractionResolution) -> wit::InteractionResolution {
    match resolution {
        InteractionResolution::Selected { option_id } => {
            wit::InteractionResolution::Selected(option_id)
        }
        InteractionResolution::Text { text } => wit::InteractionResolution::Text(text),
        InteractionResolution::Cancelled => wit::InteractionResolution::Cancelled,
    }
}

pub(crate) fn event(request: PluginRequest) -> wit::Event {
    match request {
        PluginRequest::Provider { provider, request } => wit::Event::Provider(wit::ProviderCall {
            provider: provider.0,
            request: match request {
                ProviderRequest::Start(start) => wit::ProviderRequest::Start(session_start(start)),
                ProviderRequest::Input { thread, text } => {
                    wit::ProviderRequest::Input(wit::ThreadInput {
                        thread: thread.0,
                        text,
                    })
                }
                ProviderRequest::Interrupt { thread } => wit::ProviderRequest::Interrupt(thread.0),
                ProviderRequest::Stop { thread } => wit::ProviderRequest::Stop(thread.0),
                ProviderRequest::ResolveInteraction {
                    thread,
                    request_id,
                    resolution: r,
                } => wit::ProviderRequest::ResolveInteraction(wit::InteractionAnswer {
                    thread: thread.0,
                    request_id,
                    resolution: resolution(r),
                }),
                ProviderRequest::ListModels => wit::ProviderRequest::ListModels,
            },
        }),
        PluginRequest::Environment {
            environment,
            request,
        } => wit::Event::Environment(wit::EnvironmentCall {
            environment,
            request: match request {
                EnvironmentRequest::Prepare { thread, source_dir } => {
                    wit::EnvironmentRequest::Prepare(wit::EnvironmentPrepare {
                        thread: thread.0,
                        source_dir,
                    })
                }
                EnvironmentRequest::Remove { thread, cwd } => {
                    wit::EnvironmentRequest::Remove(wit::EnvironmentRemove {
                        thread: thread.0,
                        cwd,
                    })
                }
            },
        }),
    }
}
