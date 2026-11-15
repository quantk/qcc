//! Plain-text rendering for people. Scripts and agents use `--json`.

use qcc_client::protocol::methods::PluginInfo;
use qcc_domain::{
    EventPayload, InteractionRequest, Item, ItemShape, ItemStatus, Thread, ThreadEvent,
    ThreadStatus, TurnStatus,
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
        "{}  {:<6}  {:<12}  {}",
        thread.id,
        status(thread.status),
        thread.provider,
        thread.title.as_deref().unwrap_or("(untitled)")
    )
}

pub fn thread_details(thread: &Thread) -> String {
    let mut lines = vec![
        format!("thread    {}", thread.id),
        format!("title     {}", thread.title.as_deref().unwrap_or("(untitled)")),
        format!("status    {}", status(thread.status)),
        format!("provider  {}", thread.provider),
        format!("cwd       {}", thread.cwd),
    ];
    if let Some(mode) = thread.permission_mode {
        lines.push(format!("mode      {}", mode.as_str()));
    }
    if let Some(parent) = &thread.parent {
        lines.push(format!("parent    {parent}"));
    }
    lines.push(String::new());
    lines.join("\n")
}

pub fn plugin(plugin: &PluginInfo) -> String {
    let mut lines = vec![format!(
        "{} {}{}",
        plugin.id,
        plugin.version,
        if plugin.bundled { "  (bundled)" } else { "" }
    )];
    for provider in &plugin.providers {
        lines.push(format!("  provider     {}  {}", provider.id, provider.name));
        let capabilities = &provider.capabilities;
        let modes: Vec<&str> = capabilities.permission_modes.iter().map(|m| m.as_str()).collect();
        if !modes.is_empty() {
            lines.push(format!("    modes      {}", modes.join(", ")));
        }
        let mut can = Vec::new();
        for (name, on) in [("resume", capabilities.resume), ("steer", capabilities.steer)] {
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
    let permissions = &plugin.permissions;
    let mut granted = Vec::new();
    for (name, on) in
        [("deltas", permissions.deltas), ("kv", permissions.kv), ("timers", permissions.timers)]
    {
        if on {
            granted.push(name.to_owned());
        }
    }
    if !permissions.process.is_empty() {
        granted.push(format!("process({})", permissions.process.join(", ")));
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

/// One event as text; `None` for events that only matter to programs.
pub fn event(event: &ThreadEvent) -> Option<String> {
    Some(match &event.payload {
        EventPayload::InputSubmitted { text, steered: true, .. } => {
            format!("> {text}  (into the running turn)")
        }
        EventPayload::InputSubmitted { text, .. } => format!("> {text}"),
        EventPayload::InputQueued { text, .. } => format!("(queued) {text}"),
        EventPayload::InputDropped { .. } => "(a queued message was dropped)".to_owned(),
        EventPayload::PermissionModeChanged { permission_mode } => {
            format!("permission mode: {}", permission_mode.map_or("default", |mode| mode.as_str()))
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
        EventPayload::ThreadCreated { .. }
        | EventPayload::ThreadStatusChanged { .. }
        | EventPayload::SessionStarted { .. }
        | EventPayload::SessionIdentified { .. }
        | EventPayload::SessionEnded { reason: None }
        | EventPayload::TurnStarted { .. }
        | EventPayload::ItemStarted { .. }
        | EventPayload::ItemTextDelta { .. }
        | EventPayload::UsageUpdated { .. }
        | EventPayload::InteractionResolved { .. }
        | EventPayload::ProviderUnhandled { .. } => return None,
    })
}
