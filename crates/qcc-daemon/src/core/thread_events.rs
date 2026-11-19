//! Plugins that follow every thread (`thread-events`) and notifications plugins ask
//! to show the user (`notifications`).
//!
//! A thread's state is not stored anywhere: it is worked out from the projection and
//! the sessions (`Core::thread_state`). So changes are found by comparing what each
//! thread looks like after a command with what the plugins were last told.

use std::collections::HashMap;

use qcc_domain::{InteractionId, InteractionRequest, ThreadId, ThreadState};
use qcc_plugin_host::{Notification, PluginHandle, PluginRequest, ThreadChange};
use qcc_protocol::methods::{NOTIFICATION_SHOW, NotificationShowNotification};

use super::{Core, notify};

/// The longest notification title passed on, in characters.
const TITLE_MAX_CHARS: usize = 200;
/// The longest notification body passed on, in characters.
const BODY_MAX_CHARS: usize = 1_000;

/// What plugins following threads were last told, by thread. Absent while no
/// running plugin follows threads.
pub(super) type SeenThreads = Option<HashMap<ThreadId, Seen>>;

/// What plugins following threads were last told about one thread: the facts whose
/// change is worth an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    state: ThreadState,
    /// When the latest turn ended.
    turn_at: Option<u64>,
    /// The oldest request the agent waits on.
    request: Option<InteractionId>,
}

/// What changed between two looks at a thread, if anything worth telling did.
fn compare(before: Option<&Seen>, now: &Seen) -> Option<(Option<ThreadState>, bool)> {
    if before == Some(now) {
        return None;
    }
    // A thread first seen with a turn behind it (made from another's history) did
    // not just end one.
    let turn_ended = now.turn_at.is_some() && before.is_some_and(|b| b.turn_at != now.turn_at);
    Some((before.map(|b| b.state), turn_ended))
}

fn clip(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        return text;
    }
    let mut kept: String = text.chars().take(max - 1).collect();
    kept.push('…');
    kept
}

impl Core {
    fn seen(&self, id: &ThreadId) -> Option<Seen> {
        let state = self.thread_state(id)?;
        let thread = self.supervisor.thread(id)?;
        Some(Seen {
            state,
            turn_at: thread.last_turn_at,
            request: thread.pending_interactions.first().map(|pending| pending.id.clone()),
        })
    }

    fn listening_plugins(&self) -> Vec<PluginHandle> {
        self.host.plugins().iter().filter(|p| p.permissions().thread_events).cloned().collect()
    }

    /// Tell the plugins that follow threads what changed since the last look. The
    /// first look after none of them ran only takes note: they hear of changes from
    /// the moment they start, not of the threads as they were found.
    pub(super) fn report_thread_changes(&mut self) {
        let listeners = self.listening_plugins();
        if listeners.is_empty() {
            self.thread_events = None;
            return;
        }
        let first = self.thread_events.is_none();
        let mut seen = self.thread_events.take().unwrap_or_default();
        let changed: Vec<(ThreadId, Option<Seen>, Seen)> = self
            .supervisor
            .threads()
            .filter_map(|thread| {
                let now = self.seen(&thread.id)?;
                let before = seen.get(&thread.id);
                (before != Some(&now)).then(|| (thread.id.clone(), before.cloned(), now))
            })
            .collect();
        for (id, before, now) in changed {
            if !first && let Some((previous, turn_ended)) = compare(before.as_ref(), &now) {
                self.post_thread_change(&listeners, &id, previous, turn_ended);
            }
            seen.insert(id, now);
        }
        self.thread_events = Some(seen);
    }

    fn post_thread_change(
        &self,
        listeners: &[PluginHandle],
        id: &ThreadId,
        previous: Option<ThreadState>,
        turn_ended: bool,
    ) {
        let Some(summary) = self.summary(id) else { return };
        let last_message =
            (summary.state != ThreadState::Working).then(|| self.last_message(id)).flatten();
        let request =
            summary.thread.pending_interactions.first().map(|pending| match &pending.request {
                InteractionRequest::Approval { title, .. } => title.clone(),
                InteractionRequest::Question { question, .. } => question.clone(),
            });
        let change = ThreadChange { summary, previous, turn_ended, last_message, request };
        for plugin in listeners {
            if plugin.post(PluginRequest::ThreadChanged(Box::new(change.clone()))).is_err() {
                tracing::warn!(plugin = plugin.id(), "a plugin following threads is gone");
            }
        }
    }

    /// Pass a plugin's notification on to every connection that watches threads: the
    /// desktop app shows it. With none connected it is dropped.
    pub(super) fn notify_user(&mut self, plugin: &str, notification: Notification) {
        // It may have been turned off since it asked.
        if self.host.plugin(plugin).is_none() {
            return;
        }
        if self.watchers.is_empty() {
            tracing::debug!(plugin, "no client to show a notification to");
            return;
        }
        let Notification { title, body, thread, quiet_when_shown } = notification;
        let title = clip(title, TITLE_MAX_CHARS);
        let body = clip(body, BODY_MAX_CHARS);
        let thread_id = thread.filter(|id| self.supervisor.thread(id).is_some());
        self.watchers.retain(|watcher| {
            let params = NotificationShowNotification {
                subscription: watcher.subscription,
                plugin: plugin.to_owned(),
                title: title.clone(),
                body: body.clone(),
                thread_id: thread_id.clone(),
                quiet_when_shown,
            };
            notify(&watcher.out, NOTIFICATION_SHOW, params)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(state: ThreadState, turn_at: Option<u64>, request: Option<&str>) -> Seen {
        Seen { state, turn_at, request: request.map(|id| InteractionId(id.into())) }
    }

    #[test]
    fn nothing_is_told_when_nothing_changed() {
        let idle = seen(ThreadState::Idle, Some(5), None);
        assert_eq!(compare(Some(&idle), &idle.clone()), None);
    }

    #[test]
    fn a_turn_that_ended_is_told_even_when_the_state_looks_the_same() {
        // The turn ran between two looks: idle before, idle after, a later end.
        let before = seen(ThreadState::Idle, Some(5), None);
        let now = seen(ThreadState::Idle, Some(9), None);
        assert_eq!(compare(Some(&before), &now), Some((Some(ThreadState::Idle), true)));
    }

    #[test]
    fn a_state_change_without_a_turn_end_says_so() {
        let before = seen(ThreadState::Idle, Some(5), None);
        let working = seen(ThreadState::Working, Some(5), None);
        assert_eq!(compare(Some(&before), &working), Some((Some(ThreadState::Idle), false)));
        // A new thread is told with no previous state, and with no turn ended.
        assert_eq!(compare(None, &working), Some((None, false)));
    }

    #[test]
    fn another_request_while_waiting_is_told() {
        let first = seen(ThreadState::Waiting, None, Some("a"));
        let second = seen(ThreadState::Waiting, None, Some("b"));
        assert_eq!(compare(Some(&first), &second), Some((Some(ThreadState::Waiting), false)));
    }

    #[test]
    fn long_texts_are_clipped_on_a_character_boundary() {
        assert_eq!(clip("short".into(), 10), "short");
        assert_eq!(clip("ёёёёё".into(), 3), "ёё…");
    }
}
