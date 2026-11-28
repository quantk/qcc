//! Translation between the WIT types and the domain. The only place that knows both.

use qcc_domain::plausible_icon;
use qcc_domain::{
    AgentCommand, AgentSkill, Attachment, Compaction, ForkSupport, SettingField, SettingKind,
};
use qcc_domain::{
    ChangeKind, ChangeScope, ChangedFile, CommandInfo, CommandParam, DeliverMode, Delta,
    EnvironmentChanges, EnvironmentInfo, InteractionOption, InteractionRequest,
    InteractionResolution, ItemKey, ItemShape, ItemStatus, McpServer, McpServerStatus, McpState,
    McpTransport, ModelInfo, OptionKind, PermissionMode, ProviderCapabilities, ProviderInfo,
    Revision, Role, SkillInfo, TextChannel, ThreadId, ThreadState, TokenUsage, TranscriptEntry,
    TurnStatus, UiPlacement, UiScope, UiViewInfo, UsageSnapshot, UserInput,
};
use qcc_domain::{CommentAuthor, DiffSide, LineComment};
use qcc_domain::{ContextCategory, ContextEntry, ContextKind, ContextUsage};
use qcc_domain::{LimitStatus, LimitWindow, RateLimits};

use crate::api::{
    Contribution, EnvironmentRequest, NewComment, Notification, Outcome, PluginRequest,
    ProviderRequest, SessionStart, SettledThread, SpawnThread, ThreadChange, ThreadSummary,
    UiRequest,
};
use crate::bindings::qcc::plugin::comments as wit_comments;
use crate::bindings::qcc::plugin::notifications as wit_notifications;
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

fn turn_status_to_wit(status: TurnStatus) -> wit::TurnStatus {
    match status {
        TurnStatus::Completed => wit::TurnStatus::Completed,
        TurnStatus::Failed => wit::TurnStatus::Failed,
        TurnStatus::Interrupted => wit::TurnStatus::Interrupted,
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
        wit::ItemShape::Tool(t) => ItemShape::Tool {
            name: t.name,
            input: json(&t.input)?,
            output: t.output,
            mcp_server: t.mcp_server,
            skill: t.skill,
        },
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
            point: b.point,
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
        wit::Delta::McpRemoved(name) => Delta::McpRemoved { name },
        wit::Delta::ContextUsage(c) => Delta::ContextUsage { context: context_usage(c) },
        wit::Delta::ContextCompacted(c) => Delta::ContextCompacted {
            compaction: Compaction {
                automatic: c.automatic,
                tokens_before: c.tokens_before,
                tokens_after: c.tokens_after,
            },
        },
        wit::Delta::RateLimits(limits) => Delta::RateLimits { limits: rate_limits(limits) },
        wit::Delta::ProviderError(message) => Delta::ProviderError { message },
        wit::Delta::ProviderWarning(message) => Delta::ProviderWarning { message },
        wit::Delta::Unhandled(payload) => Delta::Unhandled { payload: json(&payload)? },
    })
}

fn rate_limits(limits: wit::RateLimits) -> RateLimits {
    RateLimits {
        plan: limits.plan,
        windows: limits
            .windows
            .into_iter()
            .map(|window| LimitWindow {
                id: window.id,
                label: window.label,
                scope: window.scope,
                used_percent: window.used_percent,
                resets_at: window.resets_at,
                window_minutes: window.window_minutes,
                status: window.status.map(|status| match status {
                    wit::LimitStatus::Allowed => LimitStatus::Allowed,
                    wit::LimitStatus::Warning => LimitStatus::Warning,
                    wit::LimitStatus::Exceeded => LimitStatus::Exceeded,
                }),
            })
            .collect(),
        complete: limits.complete,
    }
}

fn context_usage(context: wit::ContextUsage) -> ContextUsage {
    ContextUsage {
        used: context.used,
        window: context.window,
        estimated: context.estimated,
        categories: context
            .categories
            .into_iter()
            .map(|category| ContextCategory {
                id: category.id,
                name: category.name,
                tokens: category.tokens,
                kind: match category.kind {
                    wit::ContextKind::Used => ContextKind::Used,
                    wit::ContextKind::Free => ContextKind::Free,
                    wit::ContextKind::Buffer => ContextKind::Buffer,
                    wit::ContextKind::Deferred => ContextKind::Deferred,
                },
            })
            .collect(),
        entries: context
            .entries
            .into_iter()
            .map(|entry| ContextEntry {
                category: entry.category,
                name: entry.name,
                tokens: entry.tokens,
            })
            .collect(),
    }
}

pub(crate) fn contribution(contribution: wit::Contribution) -> Contribution {
    match contribution {
        wit::Contribution::Provider(p) => Contribution::Provider(ProviderInfo {
            id: p.id.into(),
            name: p.name,
            capabilities: ProviderCapabilities {
                resume: p.capabilities.resume,
                steer: p.capabilities.steer,
                mcp_live: p.capabilities.mcp_live,
                compact: p.capabilities.compact,
                list_skills: p.capabilities.list_skills,
                list_commands: p.capabilities.list_commands,
                context_usage: p.capabilities.context_usage,
                images: p.capabilities.images,
                rate_limits: p.capabilities.rate_limits,
                fork: fork_from_wit(p.capabilities.fork),
                permission_modes: p
                    .capabilities
                    .permission_modes
                    .into_iter()
                    .map(mode_from_wit)
                    .collect(),
            },
            // A mark the app could not draw is better left out than shown broken.
            icon: p.icon.filter(|icon| plausible_icon(icon)),
        }),
        wit::Contribution::Environment(e) => Contribution::Environment(EnvironmentInfo {
            id: e.id,
            name: e.name,
            inspects_directories: e.inspects_directories,
            custom_placement: e.custom_placement,
        }),
        wit::Contribution::UiView(view) => Contribution::UiView(UiViewInfo {
            id: view.id,
            title: view.title,
            placement: match view.placement {
                wit::UiPlacement::App => UiPlacement::App,
                wit::UiPlacement::ThreadTab => UiPlacement::ThreadTab,
                wit::UiPlacement::ThreadPanel => UiPlacement::ThreadPanel,
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
                    positional: p.positional,
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
        wit::PermissionMode::ReadOnly => PermissionMode::ReadOnly,
        wit::PermissionMode::Ask => PermissionMode::Ask,
        wit::PermissionMode::AcceptEdits => PermissionMode::AcceptEdits,
        wit::PermissionMode::Auto => PermissionMode::Auto,
        wit::PermissionMode::Full => PermissionMode::Full,
    }
}

fn mode_to_wit(mode: PermissionMode) -> wit::PermissionMode {
    match mode {
        PermissionMode::ReadOnly => wit::PermissionMode::ReadOnly,
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
        state: thread_state(state),
        last_turn: thread.last_turn_status.map(turn_status_to_wit),
    }
}

fn thread_state(state: ThreadState) -> wit::ThreadState {
    match state {
        ThreadState::Working => wit::ThreadState::Working,
        ThreadState::Waiting => wit::ThreadState::Waiting,
        ThreadState::Idle => wit::ThreadState::Idle,
        ThreadState::Failed => wit::ThreadState::Failed,
        ThreadState::Archived => wit::ThreadState::Archived,
    }
}

pub(crate) fn notification(notification: wit_notifications::Notification) -> Notification {
    Notification {
        title: notification.title,
        body: notification.body,
        thread: notification.thread.map(ThreadId),
        quiet_when_shown: notification.quiet_when_shown,
    }
}

fn changes_to_wit(changes: EnvironmentChanges) -> wit::EnvironmentChanges {
    wit::EnvironmentChanges {
        branch: changes.branch,
        base: changes.base,
        files: changes
            .files
            .into_iter()
            .map(|file| wit::ChangedFile {
                path: file.path,
                kind: match file.kind {
                    ChangeKind::Added => wit::ChangeKind::Added,
                    ChangeKind::Modified => wit::ChangeKind::Modified,
                    ChangeKind::Deleted => wit::ChangeKind::Deleted,
                    ChangeKind::Untracked => wit::ChangeKind::Untracked,
                },
                additions: file.additions,
                deletions: file.deletions,
            })
            .collect(),
        revisions: changes
            .revisions
            .into_iter()
            .map(|r| wit::Revision { id: r.id, title: r.title, time: r.time })
            .collect(),
    }
}

fn model_info(model: ModelInfo) -> wit::ModelInfo {
    wit::ModelInfo {
        id: model.id,
        name: model.name,
        description: model.description,
        is_default: model.is_default,
        efforts: model.efforts,
        default_effort: model.default_effort,
    }
}

pub(crate) fn provider_info(info: ProviderInfo) -> wit::ProviderInfo {
    let c = info.capabilities;
    wit::ProviderInfo {
        id: info.id.0,
        name: info.name,
        capabilities: wit::ProviderCapabilities {
            resume: c.resume,
            steer: c.steer,
            permission_modes: c.permission_modes.into_iter().map(mode_to_wit).collect(),
            mcp_live: c.mcp_live,
            compact: c.compact,
            list_skills: c.list_skills,
            list_commands: c.list_commands,
            context_usage: c.context_usage,
            images: c.images,
            rate_limits: c.rate_limits,
            fork: match c.fork {
                ForkSupport::None => wit::ForkSupport::None,
                ForkSupport::Latest => wit::ForkSupport::Latest,
                ForkSupport::Any => wit::ForkSupport::Any,
            },
        },
        icon: info.icon,
    }
}

fn fork_from_wit(fork: wit::ForkSupport) -> ForkSupport {
    match fork {
        wit::ForkSupport::None => ForkSupport::None,
        wit::ForkSupport::Latest => ForkSupport::Latest,
        wit::ForkSupport::Any => ForkSupport::Any,
    }
}

fn author_from_wit(plugin: &str, author: wit_comments::CommentAuthor) -> CommentAuthor {
    CommentAuthor {
        plugin: plugin.to_owned(),
        name: author.name,
        thread: author.thread.map(ThreadId),
        severity: author.severity,
        stale: author.stale,
    }
}

/// A comment as a plugin leaves it; `plugin` is the caller, which the host knows.
pub(crate) fn new_comment(plugin: &str, comment: wit_comments::NewComment) -> NewComment {
    NewComment {
        thread: ThreadId(comment.thread),
        path: comment.path,
        line: comment.line,
        side: match comment.side {
            wit_comments::DiffSide::Old => DiffSide::Old,
            wit_comments::DiffSide::New => DiffSide::New,
        },
        quote: comment.quote,
        text: comment.text,
        author: author_from_wit(plugin, comment.author),
    }
}

pub(crate) fn line_comment(comment: LineComment) -> wit_comments::LineComment {
    let (plugin, author) = match comment.author {
        Some(author) => (
            Some(author.plugin),
            Some(wit_comments::CommentAuthor {
                name: author.name,
                thread: author.thread.map(|thread| thread.0),
                severity: author.severity,
                stale: author.stale,
            }),
        ),
        None => (None, None),
    };
    wit_comments::LineComment {
        id: comment.id.0,
        thread: comment.thread_id.0,
        path: comment.path,
        line: comment.line,
        side: match comment.side {
            DiffSide::Old => wit_comments::DiffSide::Old,
            DiffSide::New => wit_comments::DiffSide::New,
        },
        quote: comment.quote,
        text: comment.text,
        created_at: comment.created_at,
        resolved_at: comment.resolved_at,
        resolution: comment.resolution,
        plugin,
        author,
        sent_at: comment.sent_at,
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
        wit::Outcome::Commands(commands) => Outcome::Commands(
            commands
                .into_iter()
                .map(|c| AgentCommand {
                    name: c.name,
                    description: c.description,
                    argument_hint: c.argument_hint,
                    source: c.source,
                })
                .collect(),
        ),
        wit::Outcome::RateLimits(limits) => Outcome::RateLimits(rate_limits(limits)),
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
                revisions: changes
                    .revisions
                    .into_iter()
                    .map(|revision| Revision {
                        id: revision.id,
                        title: revision.title,
                        time: revision.time,
                    })
                    .collect(),
            })
        }
        wit::Outcome::EnvironmentDelivered(summary) => Outcome::EnvironmentDelivered(summary),
        wit::Outcome::EnvironmentDiff(diff) => {
            Outcome::EnvironmentDiff { diff: diff.diff, truncated: diff.truncated }
        }
        wit::Outcome::CommandResult(result) => Outcome::CommandResult {
            message: result.message,
            thread: result.thread.map(ThreadId),
            data: result.data,
        },
        // The actor resolves deferred answers; one never leaves it.
        wit::Outcome::Deferred(_) => Outcome::Done,
    }
}

/// An attachment as a provider gets it. The bytes of an image go along only to a
/// provider that looks at images; one that cannot be read goes by its path alone.
fn attachment(attachment: Attachment, images: bool) -> wit::Attachment {
    let data = if images && attachment.is_image() {
        std::fs::read(&attachment.path)
            .inspect_err(|error| {
                tracing::warn!("cannot read attachment {}: {error}", attachment.path);
            })
            .ok()
    } else {
        None
    };
    wit::Attachment {
        name: attachment.name,
        mime: attachment.mime,
        path: attachment.path,
        size: attachment.size,
        data,
    }
}

fn attachments(attachments: Vec<Attachment>, images: bool) -> Vec<wit::Attachment> {
    attachments.into_iter().map(|item| attachment(item, images)).collect()
}

fn thread_input(thread: ThreadId, input: UserInput, images: bool) -> wit::ThreadInput {
    wit::ThreadInput {
        thread: thread.0,
        text: input.text,
        notes: input.notes,
        attachments: attachments(input.attachments, images),
    }
}

fn session_start(start: SessionStart, images: bool) -> wit::SessionStart {
    let (prompt, prompt_notes, prompt_attachments) = match start.prompt {
        Some(UserInput { text, notes, attachments: files }) => {
            (Some(text), notes, attachments(files, images))
        }
        None => (None, Vec::new(), Vec::new()),
    };
    wit::SessionStart {
        thread: start.thread.0,
        cwd: start.cwd,
        model: start.model,
        effort: start.effort,
        resume: start.resume,
        fork_at: start.fork_at,
        prompt,
        prompt_notes,
        prompt_attachments,
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

/// `images` tells whether a provider of the plugin declared that it looks at images.
pub(crate) fn event(request: PluginRequest, images: impl Fn(&str) -> bool) -> wit::Event {
    match request {
        PluginRequest::Provider { provider, request } => wit::Event::Provider(wit::ProviderCall {
            request: match request {
                ProviderRequest::Start(start) => {
                    wit::ProviderRequest::Start(session_start(*start, images(&provider.0)))
                }
                ProviderRequest::Input { thread, input } => {
                    wit::ProviderRequest::Input(thread_input(thread, input, images(&provider.0)))
                }
                ProviderRequest::Steer { thread, input } => {
                    wit::ProviderRequest::Steer(thread_input(thread, input, images(&provider.0)))
                }
                ProviderRequest::Interrupt { thread } => wit::ProviderRequest::Interrupt(thread.0),
                ProviderRequest::Stop { thread } => wit::ProviderRequest::Stop(thread.0),
                ProviderRequest::Compact { thread } => wit::ProviderRequest::Compact(thread.0),
                ProviderRequest::SetMcpServers { thread, servers } => {
                    wit::ProviderRequest::SetMcpServers(wit::McpServersUpdate {
                        thread: thread.0,
                        servers: servers.into_iter().map(mcp_server).collect(),
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
                ProviderRequest::ListSkills { cwd } => wit::ProviderRequest::ListSkills(cwd),
                ProviderRequest::ListCommands { cwd } => wit::ProviderRequest::ListCommands(cwd),
                ProviderRequest::ReadRateLimits => wit::ProviderRequest::ReadRateLimits,
            },
            provider: provider.0,
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
        PluginRequest::TurnEnded { summary, status } => wit::Event::TurnEnded(wit::TurnEnded {
            thread: thread_info(*summary),
            status: turn_status_to_wit(status),
        }),
        PluginRequest::CommentsChanged(thread) => wit::Event::CommentsChanged(thread.0),
        PluginRequest::ThreadChanges { request, thread, answer } => {
            wit::Event::ThreadChanges(wit::ThreadChanges {
                request,
                thread: thread.0,
                answer: answer.map(changes_to_wit),
            })
        }
        PluginRequest::ProviderModels { request, provider, answer } => {
            wit::Event::ProviderModels(wit::ProviderModels {
                request,
                provider: provider.0,
                answer: answer.map(|models| models.into_iter().map(model_info).collect()),
            })
        }
        PluginRequest::ThreadDiff { request, thread, answer } => {
            wit::Event::ThreadDiff(wit::ThreadDiff {
                request,
                thread: thread.0,
                answer: answer.map(|(diff, truncated)| wit::EnvironmentDiff { diff, truncated }),
            })
        }
        PluginRequest::ThreadChanged(change) => {
            let ThreadChange { summary, previous, turn_ended, last_message, request } = *change;
            wit::Event::ThreadChanged(wit::ThreadChange {
                thread: thread_info(summary),
                previous: previous.map(thread_state),
                turn_ended,
                last_message,
                request,
            })
        }
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
                        dir,
                        branch,
                        reuse_branch,
                    } => wit::EnvironmentRequest::Prepare(wit::EnvironmentPrepare {
                        thread: thread.0,
                        source_dir,
                        root,
                        base,
                        carry_changes,
                        dir,
                        branch,
                        reuse_branch,
                    }),
                    EnvironmentRequest::Remove { thread, cwd, force } => {
                        wit::EnvironmentRequest::Remove(wit::EnvironmentRemove {
                            thread: thread.0,
                            cwd,
                            force,
                        })
                    }
                    EnvironmentRequest::Changes { thread, cwd, inspect, scope } => {
                        wit::EnvironmentRequest::Changes(wit::EnvironmentQuery {
                            thread: thread.0,
                            cwd,
                            inspect,
                            paths: Vec::new(),
                            scope: change_scope(scope),
                            full: false,
                        })
                    }
                    EnvironmentRequest::Diff { thread, cwd, inspect, paths, scope, full } => {
                        wit::EnvironmentRequest::Diff(wit::EnvironmentQuery {
                            thread: thread.0,
                            cwd,
                            inspect,
                            paths,
                            scope: change_scope(scope),
                            full,
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

fn change_scope(scope: ChangeScope) -> wit::ChangeScope {
    match scope {
        ChangeScope::All => wit::ChangeScope::All,
        ChangeScope::Unsaved => wit::ChangeScope::Unsaved,
        ChangeScope::Revisions { from, to } => {
            wit::ChangeScope::Revisions(wit::RevisionRange { from, to })
        }
    }
}
