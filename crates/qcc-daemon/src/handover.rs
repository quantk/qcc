//! What an agent that takes over a thread is told: it cannot resume the previous
//! agent's session, so its first message carries the conversation so far.

use qcc_domain::{Role, TranscriptEntry};

/// Long replies are cut in the middle; the start and the end say the most.
const MESSAGE_MAX_CHARS: usize = 6_000;
/// Older messages are left out first.
const CONVERSATION_MAX_CHARS: usize = 60_000;

/// Why a thread's new session cannot continue the old one.
pub(crate) enum Reason {
    /// Another agent took over; its name.
    OtherAgent(String),
    /// The thread moved into a directory of its own, with the work so far.
    Moved { cwd: String },
}

/// The first message of the new session. `history` is the conversation before
/// `message`, oldest first.
pub(crate) fn prompt(reason: &Reason, history: &[TranscriptEntry], message: &str) -> String {
    if history.is_empty() {
        return message.to_owned();
    }
    let mut kept = Vec::new();
    let mut size = 0;
    for entry in history.iter().rev() {
        let speaker = match entry.role {
            Role::User => "user",
            Role::Agent => "agent",
        };
        let text = clip(entry.text.trim(), MESSAGE_MAX_CHARS);
        let block = format!("<{speaker}>\n{text}\n</{speaker}>");
        if size + block.len() > CONVERSATION_MAX_CHARS && !kept.is_empty() {
            break;
        }
        size += block.len();
        kept.push(block);
    }
    let left_out = history.len() - kept.len();
    kept.reverse();
    let mut conversation = kept.join("\n\n");
    if left_out > 0 {
        conversation = format!("({left_out} earlier messages left out)\n\n{conversation}");
    }
    let (opening, files) = match reason {
        Reason::OtherAgent(previous) => (
            format!(
                "You are taking over this conversation from another coding agent ({previous}), \
                 in the same working directory. Its session cannot be continued"
            ),
            "The files are as that agent left them.".to_owned(),
        ),
        Reason::Moved { cwd } => (
            "This conversation moved into a working directory of its own, a separate copy of \
             the project, and its earlier session could not come along"
                .to_owned(),
            format!(
                "Work in {cwd} from now on: it holds the work so far. Leave the previous \
                 directory alone."
            ),
        ),
    };
    format!(
        "{opening}, so here is the conversation so far:\n\n<conversation>\n{conversation}\n\
         </conversation>\n\n{files} Go on from here. The user's new message:\n\n{message}"
    )
}

fn clip(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let half = max / 2;
    let head: String = text.chars().take(half).collect();
    let tail: String = text.chars().skip(count - half).collect();
    format!("{head}\n[… {} characters left out …]\n{tail}", count - 2 * half)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(role: Role, text: &str) -> TranscriptEntry {
        TranscriptEntry { role, text: text.into() }
    }

    #[test]
    fn a_thread_without_history_gets_the_message_as_it_is() {
        assert_eq!(prompt(&Reason::OtherAgent("echo".into()), &[], "hi"), "hi");
    }

    #[test]
    fn the_new_agent_reads_the_conversation_before_the_message() {
        let history = [entry(Role::User, "fix the bug"), entry(Role::Agent, "fixed in a.rs")];
        let text = prompt(&Reason::OtherAgent("Claude Code".into()), &history, "now add a test");
        assert!(text.contains("(Claude Code)"));
        assert!(text.contains("<user>\nfix the bug\n</user>\n\n<agent>\nfixed in a.rs\n</agent>"));
        assert!(text.ends_with("now add a test"));
    }

    #[test]
    fn a_thread_that_moved_is_told_where_it_works_now() {
        let history = [entry(Role::User, "fix the bug")];
        let text = prompt(&Reason::Moved { cwd: "/wt/app".into() }, &history, "go on");
        assert!(text.contains("Work in /wt/app from now on"));
        assert!(text.ends_with("go on"));
    }

    #[test]
    fn a_long_conversation_keeps_its_latest_messages() {
        let long = "x".repeat(MESSAGE_MAX_CHARS * 3);
        let history: Vec<_> = (0..40).map(|_| entry(Role::Agent, &long)).collect();
        let text = prompt(&Reason::OtherAgent("a".into()), &history, "go");
        assert!(text.len() < CONVERSATION_MAX_CHARS + 2_000);
        assert!(text.contains("earlier messages left out"));
        assert!(text.contains("characters left out"));
    }
}
