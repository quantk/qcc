//! Translation between the WIT types and the domain. The only place that knows both.

use qcc_domain::{AgentSkill, Compaction, SettingField, SettingKind};
use qcc_domain::{
    ChangeKind, ChangedFile, CommandInfo, CommandParam, DeliverMode, Delta, EnvironmentChanges,
    EnvironmentInfo, InteractionOption, InteractionRequest, InteractionResolution, ItemKey,
    ItemShape, ItemStatus, McpServer, McpServerStatus, McpState, McpTransport, ModelInfo,
    OptionKind, PermissionMode, ProviderCapabilities, ProviderInfo, Role, SkillInfo, TextChannel,
    ThreadId, ThreadState, TokenUsage, TranscriptEntry, TurnStatus, UiPlacement, UiScope,
    UiViewInfo, UsageSnapshot, UserInput,
};

use crate::api::{
    Contribution, EnvironmentRequest, Outcome, PluginRequest, ProviderRequest, SessionStart,
    SettledThread, SpawnThread, ThreadSummary, UiRequest,
};
use crate::bindings::qcc::plugin::threads as wit_threads;
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
        wit::ItemShape::FileChange(c) => ItemShape::FileChange { path: c.path, diff: c.diff },
        wit::ItemShape::FileRead(path) => ItemShape::FileRead { path },
        wit::ItemShape::Search(query) => ItemShape::Search { query },
        wit::ItemShape::Tool(t) => {
            ItemShape::Tool { name: t.name, input: json(&t.input)?, output: t.output }
        }
        wit::ItemShape::Extension(e) => {
            ItemShape::Extension { kind: e.kind, payload: json(&e.payload)? }
        }
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
        wit::Delta::ItemOpen(o) => Delta::ItemOpen { key: key(o.key), shape: shape(o.shape)? },
        wit::Delta::ItemClose(c) => Delta::ItemClose {
            key: key(c.key),
            shape: shape(c.shape)?,
            status: item_status(c.status),
        },
        wit::Delta::ItemTextDelta(d) => {
            Delta::ItemTextDelta { key: key(d.key), channel: channel(d.channel), text: d.text }
        }
        wit::Delta::ItemTextClose(c) => {
            Delta::ItemTextClose { key: key(c.key), channel: channel(c.channel), text: c.text }
        }
        wit::Delta::Usage(u) => Delta::Usage {
            usage: UsageSnapshot {
                total: token_usage(u.total),
                last: token_usage(u.last),
                model_context_window: u.model_context_window,
                context_used: u.context_used,
            },
        },
        wit::Delta::InteractionRequest(ask) => Delta::InteractionRequest {
            request_id: ask.request_id,
            item: ask.item.map(key),
            request: interaction_request(ask.request),
        },
        wit::Delta::SessionIdentity(provider_session) => {
            Delta::SessionIdentity { provider_session }
        }
        wit::Delta::SessionReset => Delta::SessionReset,
        wit::Delta::SessionEnded(reason) => Delta::SessionEnded { reason },
        wit::Delta::McpStatus(status) => Delta::McpStatus {
            server: McpServerStatus {
                name: status.name,
                state: match status.state {
                    wit::McpState::Starting => McpState::Starting,
                    wit::McpState::Ready => McpState::Ready,
                    wit::McpState::Failed => McpState::Failed,
                    wit::McpState::NeedsAuth => McpState::NeedsAuth,
                    wit::McpState::Disabled => McpState::Disabled,
                },
                error: status.error,
            },
        },
        wit::Delta::ContextCompacted(c) => Delta::ContextCompacted {
            compaction: Compaction {
                automatic: c.automatic,
                tokens_before: c.tokens_before,
                tokens_after: c.tokens_after,
            },
        },
        wit::Delta::ProviderError(message) => Delta::ProviderError { message },
        wit::Delta::ProviderWarning(message) => Delta::ProviderWarning { message },
        wit::Delta::Unhandled(payload) => Delta::Unhandled { payload: json(&payload)? },
    })
}

pub(crate) fn contribution(contribution: wit::Contribution) -> Contribution {
    match contribution {
        wit::Contribution::Provider(p) => Contribution::Provider(ProviderInfo {
            id: p.id.into(),
            name: p.name,
            capabilities: ProviderCapabilities {
                resume: p.capabilities.resume,
                steer: p.capabilities.steer,
                mcp_toggle: p.capabilities.mcp_toggle,
                compact: p.capabilities.compact,
                list_skills: p.capabilities.list_skills,
                permission_modes: p
                    .capabilities
                    .permission_modes
                    .into_iter()
                    .map(mode_from_wit)
                    .collect(),
            },
        }),
        wit::Contribution::Environment(e) => {
            Contribution::Environment(EnvironmentInfo { id: e.id, name: e.name })
        }
        wit::Contribution::UiView(view) => Contribution::UiView(UiViewInfo {
            id: view.id,
            title: view.title,
            placement: match view.placement {
                wit::UiPlacement::App => UiPlacement::App,
                wit::UiPlacement::ThreadTab => UiPlacement::ThreadTab,
            },
            scope: match view.scope {
                wit::UiScope::AllThreads => UiScope::AllThreads,
                wit::UiScope::OwnThreads => UiScope::OwnThreads,
            },
            web: view.web,
        }),
        wit::Contribution::UiItem(kind) => Contribution::UiItem(kind),
        wit::Contribution::Command(c) => Contribution::Command(CommandInfo {
            id: c.id,
            title: c.title,
            description: c.description,
            thread: c.thread,
            params: c
                .params
                .into_iter()
                .map(|p| CommandParam {
                    name: p.name,
                    label: p.label,
                    required: p.required,
                    placeholder: p.placeholder,
                })
                .collect(),
        }),
        wit::Contribution::Skill(s) => Contribution::Skill(SkillInfo {
            name: s.name,
            description: s.description,
            body: s.body,
            on_request: s.on_request,
        }),
        wit::Contribution::Setting(field) => Contribution::Setting(SettingField {
            id: field.id,
            label: field.label,
            description: field.description,
            kind: match field.kind {
                wit::SettingKind::Text => SettingKind::Text,
                wit::SettingKind::Flag => SettingKind::Flag,
                wit::SettingKind::Choice(options) => SettingKind::Choice { options },
                wit::SettingKind::Provider => SettingKind::Provider,
                wit::SettingKind::Model(provider) => SettingKind::Model { provider },
                wit::SettingKind::Effort((provider, model)) => {
                    SettingKind::Effort { provider, model }
                }
                wit::SettingKind::PermissionMode => SettingKind::PermissionMode,
            },
            default: field.default,
        }),
    }
}

fn mode_from_wit(mode: wit::PermissionMode) -> PermissionMode {
    match mode {
        wit::PermissionMode::Ask => PermissionMode::Ask,
        wit::PermissionMode::AcceptEdits => PermissionMode::AcceptEdits,
        wit::PermissionMode::Auto => PermissionMode::Auto,
        wit::PermissionMode::Full => PermissionMode::Full,
    }
}

fn mode_to_wit(mode: PermissionMode) -> wit::PermissionMode {
    match mode {
        PermissionMode::Ask => wit::PermissionMode::Ask,
        PermissionMode::AcceptEdits => wit::PermissionMode::AcceptEdits,
        PermissionMode::Auto => wit::PermissionMode::Auto,
        PermissionMode::Full => wit::PermissionMode::Full,
    }
}

pub(crate) fn spawn_options(options: wit_threads::SpawnOptions) -> SpawnThread {
    SpawnThread {
        provider: options.provider.into(),
        cwd: options.cwd,
        prompt: options.prompt,
        model: options.model,
        effort: options.effort,
        parent: options.parent.map(ThreadId),
        environment: options.environment,
        base: options.base,
        permission_mode: options.permission_mode.map(mode_from_wit),
        skills_enabled: options.skills_enabled,
        skills_disabled: options.skills_disabled,
        hidden: options.hidden,
    }
}

pub(crate) fn thread_info(summary: ThreadSummary) -> wit::ThreadInfo {
    let ThreadSummary { thread, state } = summary;
    wit::ThreadInfo {
        id: thread.id.0,
        provider: thread.provider.0,
        cwd: thread.cwd,
        parent: thread.parent.map(|parent| parent.0),
        title: thread.title,
        hidden: thread.hidden,
        model: thread.model,
        effort: thread.effort,
        environment: thread.environment.map(|environment| environment.id),
        state: match state {
            ThreadState::Working => wit::ThreadState::Working,
            ThreadState::Waiting => wit::ThreadState::Waiting,
            ThreadState::Idle => wit::ThreadState::Idle,
            ThreadState::Failed => wit::ThreadState::Failed,
            ThreadState::Archived => wit::ThreadState::Archived,
        },
        last_turn: thread.last_turn_status.map(|status| match status {
            TurnStatus::Completed => wit::TurnStatus::Completed,
            TurnStatus::Failed => wit::TurnStatus::Failed,
            TurnStatus::Interrupted => wit::TurnStatus::Interrupted,
        }),
    }
}

pub(crate) fn transcript(entries: Vec<TranscriptEntry>) -> Vec<wit_threads::Message> {
    entries
        .into_iter()
        .map(|entry| wit_threads::Message {
            role: match entry.role {
                Role::User => wit_threads::Role::User,
                Role::Agent => wit_threads::Role::Agent,
            },
            text: entry.text,
        })
        .collect()
}

pub(crate) fn outcome(outcome: wit::Outcome) -> Outcome {
    match outcome {
        wit::Outcome::Done => Outcome::Done,
        wit::Outcome::SessionStarted(s) => {
            Outcome::SessionStarted { provider_session: s.provider_session }
        }
        wit::Outcome::Models(models) => Outcome::Models(
            models
                .into_iter()
                .map(|m| ModelInfo {
                    id: m.id,
                    name: m.name,
                    description: m.description,
                    is_default: m.is_default,
                    efforts: m.efforts,
                    default_effort: m.default_effort,
                })
                .collect(),
        ),
        wit::Outcome::Skills(skills) => Outcome::Skills(
            skills
                .into_iter()
                .map(|s| AgentSkill {
                    name: s.name,
                    description: s.description,
                    source: s.source,
                    path: s.path,
                })
                .collect(),
        ),
        wit::Outcome::EnvironmentReady(cwd) => Outcome::EnvironmentReady { cwd },
        wit::Outcome::UiTree(tree) => Outcome::UiTree(tree),
        wit::Outcome::EnvironmentChanges(changes) => {
            Outcome::EnvironmentChanges(EnvironmentChanges {
                branch: changes.branch,
                base: changes.base,
                files: changes
                    .files
                    .into_iter()
                    .map(|file| ChangedFile {
                        path: file.path,
                        kind: match file.kind {
                            wit::ChangeKind::Added => ChangeKind::Added,
                            wit::ChangeKind::Modified => ChangeKind::Modified,
                            wit::ChangeKind::Deleted => ChangeKind::Deleted,
                            wit::ChangeKind::Untracked => ChangeKind::Untracked,
                        },
                        additions: file.additions,
                        deletions: file.deletions,
                    })
                    .collect(),
            })
        }
        wit::Outcome::EnvironmentDelivered(summary) => Outcome::EnvironmentDelivered(summary),
        wit::Outcome::EnvironmentDiff(diff) => {
            Outcome::EnvironmentDiff { diff: diff.diff, truncated: diff.truncated }
        }
        wit::Outcome::CommandResult(result) => {
            Outcome::CommandResult { message: result.message, thread: result.thread.map(ThreadId) }
        }
        // The actor resolves deferred answers; one never leaves it.
        wit::Outcome::Deferred(_) => Outcome::Done,
    }
}

fn thread_input(thread: ThreadId, input: UserInput) -> wit::ThreadInput {
    wit::ThreadInput { thread: thread.0, text: input.text, notes: input.notes }
}

fn session_start(start: SessionStart) -> wit::SessionStart {
    wit::SessionStart {
        thread: start.thread.0,
        cwd: start.cwd,
        model: start.model,
        effort: start.effort,
        resume: start.resume,
        prompt_notes: start.prompt.as_ref().map(|prompt| prompt.notes.clone()).unwrap_or_default(),
        prompt: start.prompt.map(|prompt| prompt.text),
        permission_mode: start.permission_mode.map(mode_to_wit),
        env: start.env,
        skills: start
            .skills
            .into_iter()
            .map(|skill| wit::SkillRef {
                name: skill.name,
                description: skill.description,
                path: skill.path,
            })
            .collect(),
        extra_dirs: start.extra_dirs,
        mcp_servers: start.mcp_servers.into_iter().map(mcp_server).collect(),
        mcp_disabled: start.mcp_disabled,
    }
}

fn mcp_server(server: McpServer) -> wit::McpServer {
    let transport = match server.transport {
        McpTransport::Stdio { command, args, env } => wit::McpTransport::Stdio(wit::McpStdio {
            command,
            args,
            env: env.into_iter().collect(),
        }),
        McpTransport::Http { url, headers } => {
            wit::McpTransport::Http(wit::McpHttp { url, headers: headers.into_iter().collect() })
        }
    };
    wit::McpServer { name: server.name, transport }
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
                ProviderRequest::Start(start) => wit::ProviderRequest::Start(session_start(*start)),
                ProviderRequest::Input { thread, input } => {
                    wit::ProviderRequest::Input(thread_input(thread, input))
                }
                ProviderRequest::Steer { thread, input } => {
                    wit::ProviderRequest::Steer(thread_input(thread, input))
                }
                ProviderRequest::Interrupt { thread } => wit::ProviderRequest::Interrupt(thread.0),
                ProviderRequest::Stop { thread } => wit::ProviderRequest::Stop(thread.0),
                ProviderRequest::Compact { thread } => wit::ProviderRequest::Compact(thread.0),
                ProviderRequest::SetMcpServer { thread, name, enabled } => {
                    wit::ProviderRequest::SetMcpServer(wit::McpSwitch {
                        thread: thread.0,
                        name,
                        enabled,
                    })
                }
                ProviderRequest::ResolveInteraction { thread, request_id, resolution: r } => {
                    wit::ProviderRequest::ResolveInteraction(wit::InteractionAnswer {
                        thread: thread.0,
                        request_id,
                        resolution: resolution(r),
                    })
                }
                ProviderRequest::ListModels => wit::ProviderRequest::ListModels,
                ProviderRequest::ListSkills => wit::ProviderRequest::ListSkills,
            },
        }),
        PluginRequest::Ui(request) => wit::Event::Ui(match request {
            UiRequest::Render { view, thread } => {
                wit::UiRequest::Render(wit::UiRender { view, thread: thread.map(|t| t.0) })
            }
            UiRequest::Action { view, thread, action, value } => {
                wit::UiRequest::Action(wit::UiAction {
                    view,
                    thread: thread.map(|t| t.0),
                    action,
                    value,
                })
            }
            UiRequest::RenderItem { kind, payload } => {
                wit::UiRequest::RenderItem(wit::UiItem { kind, payload })
            }
        }),
        PluginRequest::Command { command, thread, args } => {
            wit::Event::Command(wit::CommandCall { command, thread: thread.map(|t| t.0), args })
        }
        PluginRequest::Settings(values) => wit::Event::Settings(values),
        PluginRequest::ThreadSettled(settled) => {
            let SettledThread { summary, last_message } = *settled;
            wit::Event::ThreadSettled(wit::SettledThread {
                thread: thread_info(summary),
                last_message,
            })
        }
        PluginRequest::Environment { environment, request } => {
            wit::Event::Environment(wit::EnvironmentCall {
                environment,
                request: match request {
                    EnvironmentRequest::Prepare {
                        thread,
                        source_dir,
                        root,
                        base,
                        carry_changes,
                    } => wit::EnvironmentRequest::Prepare(wit::EnvironmentPrepare {
                        thread: thread.0,
                        source_dir,
                        root,
                        base,
                        carry_changes,
                    }),
                    EnvironmentRequest::Remove { thread, cwd, force } => {
                        wit::EnvironmentRequest::Remove(wit::EnvironmentRemove {
                            thread: thread.0,
                            cwd,
                            force,
                        })
                    }
                    EnvironmentRequest::Changes { thread, cwd } => {
                        wit::EnvironmentRequest::Changes(wit::EnvironmentQuery {
                            thread: thread.0,
                            cwd,
                        })
                    }
                    EnvironmentRequest::Diff { thread, cwd } => {
                        wit::EnvironmentRequest::Diff(wit::EnvironmentQuery {
                            thread: thread.0,
                            cwd,
                        })
                    }
                    EnvironmentRequest::Deliver { thread, cwd, mode } => {
                        wit::EnvironmentRequest::Deliver(wit::EnvironmentDeliver {
                            thread: thread.0,
                            cwd,
                            mode: match mode {
                                DeliverMode::Apply => wit::DeliverMode::Apply,
                                DeliverMode::Merge => wit::DeliverMode::Merge,
                            },
                        })
                    }
                },
            })
        }
    }
}
