//! Plain-text rendering for people. Scripts and agents use `--json`.

use qcc_client::protocol::methods::{
    CommandContribution, PluginInfo, ThreadArchiveResult, WaitedThread,
};
use qcc_domain::{
    Attachment, ChangeKind, ContextKind, EnvironmentChanges, EnvironmentState, EventPayload,
    InteractionRequest, Item, ItemShape, ItemStatus, LimitStatus, McpState, ProviderLimits,
    SettingKind, Thread, ThreadEvent, ThreadId, ThreadState, ThreadStatus, TurnStatus,
};

fn status(status: ThreadStatus) -> &'static str {
    match status {
        ThreadStatus::Idle => "idle",
        ThreadStatus::Active => "active",
        ThreadStatus::Failed => "failed",
    }
}

pub fn thread_line(thread: &Thread) -> String {
    format!(
        "{}  {:<8}  {:<12}  {}",
        thread.id,
        if thread.archived { "archived" } else { status(thread.status) },
        thread.provider,
        thread.title.as_deref().unwrap_or("(untitled)")
    )
}

/// Threads as a tree: children under their parents, oldest first at each level.
/// A thread whose parent is not in the list stands at the top.
pub fn thread_tree(threads: &[Thread]) -> Vec<String> {
    fn walk(threads: &[Thread], parent: Option<&ThreadId>, depth: usize, out: &mut Vec<String>) {
        for thread in threads.iter().filter(|thread| {
            match (&thread.parent, parent) {
                (Some(own), Some(parent)) => own == parent,
                // A root, or an orphan whose parent was left out.
                (own, None) => own.as_ref().is_none_or(|own| !threads.iter().any(|t| &t.id == own)),
                (None, Some(_)) => false,
            }
        }) {
            out.push(format!("{}{}", "  ".repeat(depth), thread_line(thread)));
            // Ids are unique, so no thread is its own ancestor in a listing.
            if depth < 32 {
                walk(threads, Some(&thread.id), depth + 1, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(threads, None, 0, &mut out);
    out
}

/// What `thread archive` archived, deepest first and the thread asked for last.
pub fn archived(result: &ThreadArchiveResult) -> Vec<String> {
    let mut lines: Vec<String> = result
        .archived
        .iter()
        .map(|thread| {
            let stopped = result.stopped.contains(&thread.id);
            format!("{}{}", thread_line(thread), if stopped { "  (stopped)" } else { "" })
        })
        .collect();
    if result.thread.archived {
        lines.push(thread_line(&result.thread));
    }
    lines
}

/// What `thread archive` left, and why.
pub fn not_archived(result: &ThreadArchiveResult) -> Vec<String> {
    let refused = result.refused.iter().map(|r| format!("{}: {}", r.thread_id, r.message));
    let kept =
        result.kept.iter().map(|id| format!("{id}: kept, a thread under it was not archived"));
    refused.chain(kept).collect()
}

pub fn state(state: ThreadState) -> &'static str {
    match state {
        ThreadState::Working => "working",
        ThreadState::Waiting => "waiting for the user",
        ThreadState::Idle => "idle",
        ThreadState::Failed => "failed",
        ThreadState::Archived => "archived",
    }
}

pub fn waited(waited: &WaitedThread) -> String {
    let thread = &waited.thread;
    let turn = match (waited.state, thread.last_turn_status) {
        (ThreadState::Idle | ThreadState::Archived, Some(TurnStatus::Completed)) => {
            ", last turn completed"
        }
        (ThreadState::Idle | ThreadState::Archived, Some(TurnStatus::Failed)) => {
            ", last turn failed"
        }
        (ThreadState::Idle | ThreadState::Archived, Some(TurnStatus::Interrupted)) => {
            ", last turn interrupted"
        }
        _ => "",
    };
    let mut text =
        format!("== {}  {}{turn}  ({})", thread.id, state(waited.state), thread.provider);
    if let Some(message) = &waited.last_message {
        text.push_str(&format!("\n{}", message.trim_end()));
    }
    text
}

pub fn command(command: &CommandContribution) -> String {
    let info = &command.command;
    let mut usage = format!("qcc {} {}", command.plugin, info.id);
    if info.thread {
        usage.push_str(" --thread <id>");
    }
    for param in &info.params {
        let flag = if param.positional {
            format!("<{}>…", param.label.to_lowercase())
        } else {
            format!("--{} <{}>", param.name, param.label.to_lowercase())
        };
        usage.push_str(&if param.required { format!(" {flag}") } else { format!(" [{flag}]") });
    }
    format!("{usage}\n  {}", info.description)
}

pub fn thread_details(thread: &Thread) -> String {
    let mut lines = vec![
        format!("thread    {}", thread.id),
        format!("title     {}", thread.title.as_deref().unwrap_or("(untitled)")),
        format!("status    {}", status(thread.status)),
        format!("provider  {}", thread.provider),
        format!("cwd       {}", thread.cwd),
    ];
    if let Some(model) = &thread.model {
        lines.push(format!("model     {model}"));
    }
    if let Some(effort) = &thread.effort {
        lines.push(format!("effort    {effort}"));
    }
    if thread.archived {
        lines[2] = "status    archived".into();
    }
    if let Some(environment) = &thread.environment {
        let state = match environment.state {
            EnvironmentState::Pending => "pending",
            EnvironmentState::Ready => "ready",
            EnvironmentState::Failed => "failed",
            EnvironmentState::Removed => "removed",
        };
        lines.push(format!(
            "env       {} ({state}), from {}",
            environment.id, environment.source_dir
        ));
    }
    if let Some(mode) = thread.permission_mode {
        lines.push(format!("mode      {}", mode.as_str()));
    }
    if let Some(parent) = &thread.parent {
        lines.push(format!("parent    {parent}"));
    }
    if let Some(by) = &thread.started_by {
        lines.push(format!("started   by thread {by}"));
    }
    if let Some(origin) = &thread.forked_from {
        lines.push(format!("forked    from thread {} at turn {}", origin.thread, origin.turn));
    }
    if let Some(automation) = &thread.automation {
        lines.push(format!("started   by automation {automation}"));
    }
    if thread.hidden {
        lines.push("hidden    kept out of lists".to_owned());
    }
    if !thread.skills_enabled.is_empty() {
        lines.push(format!("skills    {} on", thread.skills_enabled.join(", ")));
    }
    if !thread.skills_disabled.is_empty() {
        lines.push(format!("skills    {} off", thread.skills_disabled.join(", ")));
    }
    let mcp = mcp_servers(thread);
    if !mcp.is_empty() {
        lines.extend(mcp.lines().map(str::to_owned));
    }
    lines.push(String::new());
    lines.join("\n")
}

/// The MCP servers of the thread's session, then those it keeps off that the
/// session does not report. Empty when there are none.
/// What the context window holds, one part per line, with its share of the window.
pub fn context(thread: &Thread) -> String {
    let percent = |tokens: u64, window: Option<u64>| match window {
        Some(window) if window > 0 => format!("{:>5.1}%", tokens as f64 * 100.0 / window as f64),
        _ => String::new(),
    };
    let Some(context) = &thread.context else {
        return match (thread.usage.context_used, thread.usage.context_window) {
            (Some(used), Some(window)) => format!(
                "context  {used} of {window} tokens ({})\n(the agent does not break it down)",
                percent(used, Some(window)).trim()
            ),
            (Some(used), None) => format!("context  {used} tokens, the window unknown"),
            _ => "the agent has not said how full its context is".to_owned(),
        };
    };
    let mut lines = vec![match context.window {
        Some(window) => format!(
            "context  {} of {window} tokens ({}){}",
            context.used,
            percent(context.used, Some(window)).trim(),
            if context.estimated { ", estimated" } else { "" }
        ),
        None => format!("context  {} tokens", context.used),
    }];
    for category in &context.categories {
        let note = match category.kind {
            ContextKind::Used => "",
            ContextKind::Free => "  (free)",
            ContextKind::Buffer => "  (reserved)",
            ContextKind::Deferred => "  (deferred, not in the window)",
        };
        lines.push(format!(
            "  {:<28} {:>9} {}{note}",
            clip(&category.name, 28),
            category.tokens,
            percent(category.tokens, context.window)
        ));
    }
    lines.join("\n")
}

pub fn mcp_servers(thread: &Thread) -> String {
    let mut lines = Vec::new();
    let mut add = |name: &str, state: &str| {
        let label = if lines.is_empty() { "mcp" } else { "" };
        lines.push(format!("{label:<9} {name} {state}"));
    };
    for server in &thread.mcp_servers {
        let state = match server.state {
            McpState::Starting => "starting",
            McpState::Ready => "ready",
            McpState::Failed => "failed",
            McpState::NeedsAuth => "needs auth",
            McpState::Disabled => "off",
        };
        // Agents put whole transport dumps here; the first words say enough.
        let error = server
            .error
            .as_deref()
            .map(|error| format!(": {}", clip(error, 160)))
            .unwrap_or_default();
        let kept_off = thread.mcp_disabled.contains(&server.name);
        let pending = if kept_off && server.state != McpState::Disabled {
            " (kept off; the session has not taken it yet)"
        } else {
            ""
        };
        add(&server.name, &format!("{state}{error}{pending}"));
    }
    for name in &thread.mcp_disabled {
        if !thread.mcp_servers.iter().any(|server| &server.name == name) {
            add(name, "off");
        }
    }
    lines.join("\n")
}

/// The first line of `text`, cut to `max` characters.
fn clip(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    match line.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_owned(),
    }
}

pub fn changes(changes: &EnvironmentChanges) -> String {
    let mut lines = Vec::new();
    if let Some(branch) = &changes.branch {
        lines.push(format!("branch  {branch}"));
    }
    if let Some(base) = &changes.base {
        lines.push(format!("base    {base}"));
    }
    if changes.files.is_empty() {
        lines.push("no changes".into());
    }
    for file in &changes.files {
        let kind = match file.kind {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
            ChangeKind::Untracked => '?',
        };
        let stat = match (file.additions, file.deletions) {
            (Some(additions), Some(deletions)) => format!("  +{additions} -{deletions}"),
            _ => String::new(),
        };
        lines.push(format!("{kind}  {}{stat}", file.path));
    }
    if !changes.revisions.is_empty() {
        lines.push(String::new());
        lines.push("commits since the base, newest first:".into());
        for revision in &changes.revisions {
            let id = revision.id.get(..10).unwrap_or(&revision.id);
            lines.push(format!("{id}  {}", revision.title));
        }
    }
    lines.join("\n")
}

pub fn plugin(plugin: &PluginInfo) -> String {
    let status = match (&plugin.error, plugin.enabled) {
        (_, false) => "  disabled",
        (Some(_), true) => "  failed",
        (None, true) => "",
    };
    let mut lines = vec![format!(
        "{} {}{status}{}{}",
        plugin.id,
        plugin.version,
        if plugin.bundled { "  (bundled)" } else { "" },
        if plugin.example { "  (example)" } else { "" }
    )];
    if let Some(description) = &plugin.description {
        lines.push(format!("  {description}"));
    }
    if let Some(error) = &plugin.error {
        lines.push(format!("  error        {error}"));
    }
    for provider in &plugin.providers {
        lines.push(format!("  provider     {}  {}", provider.id, provider.name));
        let capabilities = &provider.capabilities;
        let modes: Vec<&str> = capabilities.permission_modes.iter().map(|m| m.as_str()).collect();
        if !modes.is_empty() {
            lines.push(format!("    modes      {}", modes.join(", ")));
        }
        let mut can = Vec::new();
        for (name, on) in [
            ("resume", capabilities.resume),
            ("steer", capabilities.steer),
            ("mcp_live", capabilities.mcp_live),
            ("compact", capabilities.compact),
            ("context_usage", capabilities.context_usage),
            ("images", capabilities.images),
        ] {
            if on {
                can.push(name);
            }
        }
        if !can.is_empty() {
            lines.push(format!("    can        {}", can.join(", ")));
        }
    }
    for environment in &plugin.environments {
        lines.push(format!("  environment  {}  {}", environment.id, environment.name));
    }
    for command in &plugin.commands {
        lines.push(format!("  command      {} {}  {}", plugin.id, command.id, command.title));
    }
    for skill in &plugin.skills {
        lines.push(format!("  skill        {skill}"));
    }
    let permissions = &plugin.permissions;
    let mut granted = Vec::new();
    for (name, on) in [
        ("deltas", permissions.deltas),
        ("kv", permissions.kv),
        ("timers", permissions.timers),
        ("threads", permissions.threads),
        ("skills", permissions.skills),
        ("turn-events", permissions.turn_events),
        ("comments", permissions.comments),
        ("files", permissions.files),
        ("thread-events", permissions.thread_events),
        ("notifications", permissions.notifications),
    ] {
        if on {
            granted.push(name.to_owned());
        }
    }
    if !permissions.process.is_empty() {
        granted.push(format!("process({})", permissions.process.join(", ")));
    }
    if !permissions.home_files.is_empty() {
        let dirs: Vec<String> =
            permissions.home_files.iter().map(|dir| format!("~/{dir}")).collect();
        granted.push(format!("home-files({})", dirs.join(", ")));
    }
    let granted = if granted.is_empty() { "none".to_owned() } else { granted.join(", ") };
    lines.push(format!("  permissions  {granted}"));
    lines.join("\n")
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("  {line}")).collect::<Vec<_>>().join("\n")
}

fn item(item: &Item) -> Option<String> {
    let suffix = match item.status {
        ItemStatus::Completed | ItemStatus::Pending => "",
        ItemStatus::Failed => "  (failed)",
        ItemStatus::Declined => "  (declined)",
        ItemStatus::Interrupted => "  (interrupted)",
    };
    Some(match &item.shape {
        ItemShape::AgentMessage { text } if text.trim().is_empty() => return None,
        ItemShape::AgentMessage { text } => format!("{}{suffix}", text.trim_end()),
        ItemShape::Reasoning { text } => format!("(thinking) {}", text.trim_end()),
        ItemShape::Plan { text } => format!("(plan)\n{}", indent(text.trim_end())),
        ItemShape::Command { command, output, exit_code, .. } => {
            let mut line = format!("$ {command}{suffix}");
            if let Some(code) = exit_code.filter(|code| *code != 0) {
                line.push_str(&format!("  (exit {code})"));
            }
            match output.as_deref().map(str::trim_end).filter(|o| !o.is_empty()) {
                Some(output) => format!("{line}\n{}", indent(output)),
                None => line,
            }
        }
        ItemShape::FileChange { path, .. } => format!("[edit] {path}{suffix}"),
        ItemShape::FileRead { path } => format!("[read] {path}{suffix}"),
        ItemShape::Search { query } => format!("[search] {query}{suffix}"),
        ItemShape::Tool { name, output, .. } => {
            match output.as_deref().map(str::trim_end).filter(|o| !o.is_empty()) {
                Some(output) => format!("[tool] {name}{suffix}\n{}", indent(output)),
                None => format!("[tool] {name}{suffix}"),
            }
        }
        ItemShape::Extension { kind, .. } => format!("[{kind}]{suffix}"),
    })
}

fn attachment_line(attachment: &Attachment) -> String {
    format!("  [attached {} ({}, {} bytes)]", attachment.name, attachment.mime, attachment.size)
}

/// One event as text; `None` for events that only matter to programs.
pub fn event(event: &ThreadEvent) -> Option<String> {
    Some(match &event.payload {
        EventPayload::InputSubmitted { text, notes, attachments, steered, .. } => {
            let mut lines = vec![format!("> {text}")];
            if *steered {
                lines[0].push_str("  (into the running turn)");
            }
            lines.extend(notes.iter().map(|note| format!("  (qcc: {note})")));
            lines.extend(attachments.iter().map(attachment_line));
            lines.join("\n")
        }
        EventPayload::InputQueued { text, attachments, .. } => {
            let mut lines = vec![format!("(queued) {text}")];
            lines.extend(attachments.iter().map(attachment_line));
            lines.join("\n")
        }
        EventPayload::InputDropped { .. } => "(a queued message was dropped)".to_owned(),
        EventPayload::PermissionModeChanged { permission_mode } => {
            format!("permission mode: {}", permission_mode.map_or("default", |mode| mode.as_str()))
        }
        EventPayload::AgentChanged { provider, model, effort } => {
            let mut line = format!("agent: {provider}");
            if let Some(model) = model {
                line.push_str(&format!(", model {model}"));
            }
            if let Some(effort) = effort {
                line.push_str(&format!(", effort {effort}"));
            }
            line
        }
        EventPayload::ItemCompleted { item: completed, .. } => item(completed)?,
        EventPayload::TurnCompleted { status, error, .. } => match (status, error) {
            (TurnStatus::Completed, _) => return None,
            (TurnStatus::Failed, Some(error)) => format!("turn failed: {error}"),
            (TurnStatus::Failed, None) => "turn failed".to_owned(),
            (TurnStatus::Interrupted, _) => "turn interrupted".to_owned(),
        },
        EventPayload::InteractionRequested { interaction_id, request, .. } => {
            let (headline, options, allow_text) = match request {
                InteractionRequest::Approval { title, detail, options } => {
                    let headline = match detail {
                        Some(detail) => format!("{title} {detail}"),
                        None => title.clone(),
                    };
                    (headline, options, false)
                }
                InteractionRequest::Question { question, options, allow_text } => {
                    (question.clone(), options, *allow_text)
                }
            };
            let mut answers: Vec<String> =
                options.iter().map(|option| format!("--option {}", option.id)).collect();
            if allow_text {
                answers.push("--text <answer>".to_owned());
            }
            format!(
                "? {headline}\n  answer: qcc interaction resolve {} {interaction_id} {}",
                event.thread_id,
                answers.join(" | ")
            )
        }
        EventPayload::ProviderError { message } => format!("error: {message}"),
        EventPayload::ProviderWarning { message } => format!("warning: {message}"),
        EventPayload::SessionEnded { reason: Some(reason) } => format!("session ended: {reason}"),
        EventPayload::EnvironmentReady { cwd } => format!("working directory: {cwd}"),
        EventPayload::EnvironmentRemoved => "(the working directory was removed)".to_owned(),
        EventPayload::ThreadArchived => "(archived)".to_owned(),
        EventPayload::TitleChanged { title } => format!("(renamed: {title})"),
        EventPayload::EnvironmentDetached { message } => {
            format!("could not move into the environment: {message}")
        }
        EventPayload::EnvironmentAttached { environment } => {
            format!("(moving into a new {} environment)", environment.id)
        }
        EventPayload::ProjectChanged { project } => match project {
            Some(project) => format!("(project {project})"),
            None => "(no project)".to_owned(),
        },
        EventPayload::McpDisabledChanged { disabled } => match disabled.is_empty() {
            true => "(every MCP server on)".to_owned(),
            false => format!("(MCP servers off: {})", disabled.join(", ")),
        },
        EventPayload::ConversationReset => "── a new conversation ──".to_owned(),
        EventPayload::ContextCompacted { compaction } => {
            let why = if compaction.automatic { "the agent compacted its context" } else { "context compacted" };
            match (compaction.tokens_before, compaction.tokens_after) {
                (Some(before), Some(after)) => format!("({why}: {before} → {after} tokens)"),
                _ => format!("({why})"),
            }
        }
        EventPayload::SkillsChanged { enabled, disabled } => {
            let mut parts = Vec::new();
            if !enabled.is_empty() {
                parts.push(format!("skills on: {}", enabled.join(", ")));
            }
            if !disabled.is_empty() {
                parts.push(format!("skills off: {}", disabled.join(", ")));
            }
            if parts.is_empty() {
                "(the skills every session gets)".to_owned()
            } else {
                format!("({})", parts.join("; "))
            }
        }
        EventPayload::ParentChanged { parent } => match parent {
            Some(parent) => format!("(moved under {parent})"),
            None => "(no longer under another thread)".to_owned(),
        },
        // The failure follows as a provider error, in full.
        EventPayload::EnvironmentFailed { .. }
        | EventPayload::EnvironmentPreparing
        | EventPayload::ThreadCreated { .. }
        | EventPayload::ThreadStatusChanged { .. }
        | EventPayload::SessionStarted { .. }
        | EventPayload::SessionIdentified { .. }
        | EventPayload::SessionEnded { reason: None }
        | EventPayload::TurnStarted { .. }
        | EventPayload::ItemStarted { .. }
        | EventPayload::ItemTextDelta { .. }
        | EventPayload::UsageUpdated { .. }
        | EventPayload::ContextUsageUpdated { .. }
        // `thread show` lists the servers; every change of one is not news.
        | EventPayload::McpServersChanged { .. }
        | EventPayload::InteractionResolved { .. }
        | EventPayload::ProviderUnhandled { .. } => return None,
    })
}

/// What a plugin setting holds, in a few words.
pub fn setting_kind(kind: &SettingKind) -> String {
    match kind {
        SettingKind::Text => "text".into(),
        SettingKind::Flag => "true or false".into(),
        SettingKind::Choice { options } => options.join(" | "),
        SettingKind::Provider => "an agent provider".into(),
        SettingKind::Model { provider } => format!("a model of the agent in {provider}"),
        SettingKind::Effort { model, .. } => format!("an effort level of the model in {model}"),
        SettingKind::PermissionMode => "ask | accept-edits | auto | full".into(),
    }
}

/// A span of time the way people say it: `45m`, `2h 10m`, `3d 4h`.
pub fn span(ms: u64) -> String {
    let minutes = ms / 60_000;
    match minutes {
        0 => "<1m".to_owned(),
        1..60 => format!("{minutes}m"),
        60..1440 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}

/// A provider's usage limits: a heading with the plan and how old the news is, then
/// a line per window.
pub fn limits(limits: &ProviderLimits, now: u64) -> Vec<String> {
    let plan = limits.plan.as_deref().map(|plan| format!(" ({plan})")).unwrap_or_default();
    let age = span(now.saturating_sub(limits.updated_at));
    let mut lines = vec![format!("{}{plan}  as of {age} ago", limits.provider)];
    if limits.windows.is_empty() {
        lines.push("  no usage limits on this account".to_owned());
    }
    for window in &limits.windows {
        let name = match &window.scope {
            Some(scope) => format!("{} · {scope}", window.label),
            None => window.label.clone(),
        };
        let reset = match window.resets_at {
            // The window started over since: what was spent is no longer.
            Some(at) if at <= now => "reset since".to_owned(),
            Some(at) => format!("resets in {}", span(at - now)),
            None => String::new(),
        };
        let status = match window.status {
            Some(LimitStatus::Exceeded) => "  limit reached",
            Some(LimitStatus::Warning) => "  warning",
            _ => "",
        };
        lines.push(
            format!("  {name:<20} {:>4}%  {reset}{status}", window.used_percent)
                .trim_end()
                .to_owned(),
        );
    }
    lines
}
