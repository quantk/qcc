//! What an agent that takes over a thread is told: it cannot resume the previous
//! agent's session, so its first message carries the conversation so far.

use qcc_domain::{Role, TranscriptEntry};

/// Long replies are cut in the middle; the start and the end say the most.
const MESSAGE_MAX_CHARS: usize = 6_000;
/// Older messages are left out first.
const CONVERSATION_MAX_CHARS: usize = 60_000;

/// The first message of the new agent's session. `history` is the conversation
/// before `message`, oldest first.
pub(crate) fn prompt(previous: &str, history: &[TranscriptEntry], message: &str) -> String {
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
    format!(
        "You are taking over this conversation from another coding agent ({previous}), in \
         the same working directory. Its session cannot be continued, so here is the \
         conversation so far:\n\n<conversation>\n{conversation}\n</conversation>\n\n\
         The files are as that agent left them. Go on from here. The user's new message:\n\n\
         {message}"
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
        assert_eq!(prompt("echo", &[], "hi"), "hi");
    }

    #[test]
    fn the_new_agent_reads_the_conversation_before_the_message() {
        let history = [entry(Role::User, "fix the bug"), entry(Role::Agent, "fixed in a.rs")];
        let text = prompt("Claude Code", &history, "now add a test");
        assert!(text.contains("(Claude Code)"));
        assert!(text.contains("<user>\nfix the bug\n</user>\n\n<agent>\nfixed in a.rs\n</agent>"));
        assert!(text.ends_with("now add a test"));
    }

    #[test]
    fn a_long_conversation_keeps_its_latest_messages() {
        let long = "x".repeat(MESSAGE_MAX_CHARS * 3);
        let history: Vec<_> = (0..40).map(|_| entry(Role::Agent, &long)).collect();
        let text = prompt("a", &history, "go");
        assert!(text.len() < CONVERSATION_MAX_CHARS + 2_000);
        assert!(text.contains("earlier messages left out"));
        assert!(text.contains("characters left out"));
    }
}
