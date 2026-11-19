//! Orchestration: agents that delegate to agents.
//!
//! Nothing here is part of the core. The plugin uses the same public API any plugin
//! could ask for:
//!
//! * a **skill** — instructions every agent session gets, describing the `qcc`
//!   commands an agent uses to start, wait for and read child threads;
//! * **`handoff`** — continue a thread's task with another agent (or a fresh one of
//!   the same kind), which gets the conversation so far;
//! * **`review`** — start a reviewer on what a thread did; when the reviewer is done,
//!   its findings are sent back to the thread as a message.
//!
//! Both commands are built on the `threads` import, and `review` follows its
//! reviewer with `threads.watch`.

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::threads::{Message, Role, SpawnOptions};

const SKILL_NAME: &str = "qcc-orchestration";
const SKILL_BODY: &str = include_str!("../SKILL.md");

/// Reviews in flight: `review/<reviewer thread>` holds the reviewed thread.
const REVIEW_PREFIX: &str = "review/";

/// How much of a conversation a handoff carries over.
const HANDOFF_MAX_CHARS: usize = 40_000;
/// The longest single message a handoff keeps whole.
const MESSAGE_MAX_CHARS: usize = 8_000;
const TASK_MAX_CHARS: usize = 2_000;

#[derive(Default)]
struct Orchestration {
    /// The reviews of the last daemon run are watched again: the instance lives
    /// exactly as long as the daemon, so once is enough.
    rewatched: bool,
}

/// A thread started from `source`, in its directory.
fn child_of(source: &ThreadInfo, provider: String, prompt: String) -> SpawnOptions {
    SpawnOptions {
        provider,
        cwd: Some(source.cwd.clone()),
        prompt: Some(prompt),
        model: None,
        effort: None,
        parent: Some(source.id.clone()),
        environment: None,
        base: None,
        permission_mode: None,
        skills_enabled: Vec::new(),
        skills_disabled: Vec::new(),
        hidden: false,
    }
}

fn param(name: &str, label: &str, placeholder: &str) -> CommandParam {
    CommandParam {
        name: name.into(),
        label: label.into(),
        required: false,
        placeholder: Some(placeholder.into()),
        positional: false,
    }
}

fn arg<'a>(call: &'a CommandCall, name: &str) -> Option<&'a str> {
    call.args
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
}

/// At most `max` characters of `text`, marked when cut.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept}\n[… cut short]")
}

/// The conversation as a handoff carries it: the latest messages that fit.
fn conversation(messages: &[Message]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut size = 0;
    for message in messages.iter().rev() {
        let who = match message.role {
            Role::User => "User",
            Role::Agent => "Agent",
        };
        let part = format!("### {who}\n\n{}\n", clip(message.text.trim(), MESSAGE_MAX_CHARS));
        if size + part.len() > HANDOFF_MAX_CHARS && !parts.is_empty() {
            break;
        }
        size += part.len();
        parts.push(part);
    }
    let left_out = messages.len() - parts.len();
    parts.reverse();
    let mut text = String::new();
    if left_out > 0 {
        text.push_str(&format!("[{left_out} earlier messages left out]\n\n"));
    }
    text.push_str(&parts.join("\n"));
    text
}

fn handoff_prompt(source: &ThreadInfo, messages: &[Message], note: Option<&str>) -> String {
    let next = match note {
        Some(note) => format!("What to do now: {note}"),
        None => "Continue the task from where it stopped.".to_owned(),
    };
    format!(
        "You are taking over a task from another agent: qcc thread {id}, run by {provider}. \
         You work in the same directory, {cwd}, so its changes are already there.\n\n\
         Its conversation so far, oldest first:\n\n<conversation>\n{conversation}\n</conversation>\n\n\
         {next}",
        id = source.id,
        provider = source.provider,
        cwd = source.cwd,
        conversation = conversation(messages),
    )
}

fn review_prompt(source: &ThreadInfo, task: Option<&str>, focus: Option<&str>) -> String {
    let mut prompt = format!(
        "Review the work of another agent: qcc thread {id}, which worked in {cwd}.\n\n",
        id = source.id,
        cwd = source.cwd,
    );
    if let Some(task) = task {
        prompt.push_str(&format!(
            "The task it was given:\n\n<task>\n{}\n</task>\n\n",
            clip(task.trim(), TASK_MAX_CHARS)
        ));
    }
    prompt.push_str(&format!(
        "See what it changed with `qcc thread changes {id}` and `qcc thread diff {id}`. If \
         those say the thread has no environment, look at the directory with git instead \
         (`git status`, `git diff`). `qcc thread show {id}` has its whole timeline.\n\n",
        id = source.id,
    ));
    if let Some(focus) = focus {
        prompt.push_str(&format!("Pay particular attention to: {focus}\n\n"));
    }
    prompt.push_str(
        "Do not change any files. Reply with your findings, most important first: bugs, \
         risks, missing tests and unclear code, each with the file and line. Say plainly if \
         you found nothing that needs changing. Your reply is sent to the agent as it is.",
    );
    prompt
}

fn findings_message(reviewer: &str, findings: &str) -> String {
    format!(
        "A reviewer (qcc thread {reviewer}) looked at your work and reports:\n\n{findings}\n\n\
         Address the findings you agree with, and say which ones you disagree with and why."
    )
}

/// A thread that is still at work cannot be handed on or reviewed yet.
fn settled(thread: &ThreadInfo) -> Result<(), String> {
    match thread.state {
        ThreadState::Working | ThreadState::Waiting => {
            Err(format!("thread {} is still working; wait for it or stop it first", thread.id))
        }
        ThreadState::Archived => Err(format!("thread {} is archived", thread.id)),
        ThreadState::Idle | ThreadState::Failed => Ok(()),
    }
}

impl Orchestration {
    fn handoff(&mut self, call: &CommandCall) -> Result<CommandResult, String> {
        let source = threads::get(call.thread.as_deref().ok_or("name the thread to hand off")?)?;
        settled(&source)?;
        let messages = threads::transcript(&source.id)?;
        let provider = arg(call, "to").unwrap_or(&source.provider).to_owned();
        // A model name and an effort level mean something only to the provider they
        // were given for.
        let same = provider == source.provider;
        let model = if same { source.model.clone() } else { None };
        let effort = if same { source.effort.clone() } else { None };
        let prompt = handoff_prompt(&source, &messages, arg(call, "note"));
        let child = threads::spawn(&SpawnOptions {
            model,
            effort,
            ..child_of(&source, provider.clone(), prompt)
        })?;
        Ok(CommandResult {
            message: format!(
                "Handed thread {} off to {provider} in thread {}",
                source.id, child.id
            ),
            thread: Some(child.id),
            data: None,
        })
    }

    fn review(&mut self, call: &CommandCall) -> Result<CommandResult, String> {
        let source = threads::get(call.thread.as_deref().ok_or("name the thread to review")?)?;
        settled(&source)?;
        let messages = threads::transcript(&source.id)?;
        let task = messages.iter().find(|m| m.role == Role::User).map(|m| m.text.as_str());
        let provider = arg(call, "by").unwrap_or(&source.provider).to_owned();
        let prompt = review_prompt(&source, task, arg(call, "focus"));
        let reviewer = threads::spawn(&child_of(&source, provider.clone(), prompt))?;
        kv::put(&format!("{REVIEW_PREFIX}{}", reviewer.id), source.id.as_bytes())?;
        threads::watch(&reviewer.id)?;
        Ok(CommandResult {
            message: format!(
                "{provider} reviews thread {} in thread {}; the findings go back to it when done",
                source.id, reviewer.id
            ),
            thread: Some(reviewer.id),
            data: None,
        })
    }

    /// Watches end with the daemon; the reviews they were for are kept. Watch them
    /// again: a reviewer that finished meanwhile is reported at once.
    fn rewatch_reviews(&mut self) {
        if std::mem::replace(&mut self.rewatched, true) {
            return;
        }
        let Ok(keys) = kv::keys(REVIEW_PREFIX) else { return };
        for key in keys {
            let reviewer = &key[REVIEW_PREFIX.len()..];
            if threads::watch(reviewer).is_err() {
                let _ = kv::remove(&key);
            }
        }
    }
}

impl Commands for Orchestration {
    fn commands(&self) -> Vec<CommandInfo> {
        vec![
            CommandInfo {
                id: "handoff".into(),
                title: "Hand off to another agent".into(),
                description: "Start a new thread in the same directory that continues this \
                              one's task, with its conversation so far."
                    .into(),
                thread: true,
                params: vec![
                    param("to", "Agent", "claude-code, codex, opencode; default: the same"),
                    param("note", "What to do next", "default: continue where it stopped"),
                ],
            },
            CommandInfo {
                id: "review".into(),
                title: "Review this thread's work".into(),
                description: "Start a reviewer on what this thread changed. Its findings are \
                              sent back to the thread when it is done."
                    .into(),
                thread: true,
                params: vec![
                    param("by", "Reviewer", "claude-code, codex, opencode; default: the same"),
                    param("focus", "Focus", "what to look at most closely"),
                ],
            },
        ]
    }

    fn run(&mut self, call: CommandCall) -> Result<Reply<CommandResult>, String> {
        self.rewatch_reviews();
        match call.command.as_str() {
            "handoff" => self.handoff(&call),
            "review" => self.review(&call),
            other => Err(format!("unknown command {other:?}")),
        }
        .map(Reply::Now)
    }
}

impl Plugin for Orchestration {
    fn commands(&mut self) -> Option<&mut dyn Commands> {
        Some(self)
    }

    fn skills(&self) -> Vec<SkillInfo> {
        vec![SkillInfo {
            name: SKILL_NAME.into(),
            description: "How to delegate work to other coding agents (Claude Code, Codex, \
                          opencode) through the qcc CLI: start child threads, wait for them, \
                          read their answers and changes. Read it before splitting a task \
                          between agents or when asked to use other agents."
                .into(),
            body: SKILL_BODY.into(),
            on_request: false,
        }]
    }

    fn on_thread_settled(&mut self, settled: SettledThread) {
        let reviewer = settled.thread;
        let key = format!("{REVIEW_PREFIX}{}", reviewer.id);
        let Ok(Some(source)) = kv::get(&key) else { return };
        let _ = kv::remove(&key);
        let source = String::from_utf8_lossy(&source).into_owned();
        let completed = reviewer.state == ThreadState::Idle
            && reviewer.last_turn == Some(TurnStatus::Completed);
        match settled.last_message {
            Some(findings) if completed => {
                if let Err(error) =
                    threads::send(&source, &findings_message(&reviewer.id, &findings))
                {
                    log::warn(format!("cannot send the review of {source} back: {error}"));
                }
            }
            // Stopped, failed or silent: nothing worth sending.
            _ => log::info(format!("review {} of {source} ended without findings", reviewer.id)),
        }
    }
}

qcc_plugin_sdk::export_plugin!(Orchestration);

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: Role, text: &str) -> Message {
        Message { role, text: text.into() }
    }

    #[test]
    fn a_handoff_keeps_the_latest_messages_that_fit() {
        let long = "x".repeat(MESSAGE_MAX_CHARS * 2);
        let mut messages = vec![message(Role::User, "first task")];
        for _ in 0..10 {
            messages.push(message(Role::Agent, &long));
        }
        messages.push(message(Role::User, "the latest question"));
        let text = conversation(&messages);
        assert!(text.len() <= HANDOFF_MAX_CHARS + 200, "{}", text.len());
        assert!(text.starts_with("["), "{}", &text[..80]);
        assert!(text.contains("earlier messages left out"));
        assert!(text.ends_with("### User\n\nthe latest question\n"));
        assert!(text.contains("[… cut short]"));
        assert!(!text.contains("first task"));

        let short = conversation(&messages[..1]);
        assert_eq!(short, "### User\n\nfirst task\n");
    }

    #[test]
    fn prompts_say_where_and_what() {
        let source = ThreadInfo {
            id: "thr_1".into(),
            provider: "codex".into(),
            cwd: "/work/tree".into(),
            parent: None,
            title: None,
            hidden: false,
            model: None,
            effort: None,
            environment: Some("git-worktree".into()),
            state: ThreadState::Idle,
            last_turn: Some(TurnStatus::Completed),
        };
        let handoff = handoff_prompt(&source, &[message(Role::User, "fix it")], Some("add tests"));
        assert!(handoff.contains("qcc thread thr_1, run by codex"));
        assert!(handoff.contains("/work/tree"));
        assert!(handoff.ends_with("What to do now: add tests"));

        let review = review_prompt(&source, Some("fix it"), None);
        assert!(review.contains("`qcc thread diff thr_1`"));
        assert!(review.contains("<task>\nfix it\n</task>"));
        assert!(review.contains("Do not change any files"));
        assert!(settled(&source).is_ok());
        assert!(settled(&ThreadInfo { state: ThreadState::Working, ..source }).is_err());
    }
}
