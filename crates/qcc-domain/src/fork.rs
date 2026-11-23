//! The turns of a thread as places to fork it from: what each turn ended at, in
//! which provider session, and whether a later turn of that session followed.

use crate::event::{EventPayload, ThreadEvent};
use crate::ids::{ItemId, ProviderId, TurnId};
use crate::item::ItemShape;
use crate::provider::{ForkSupport, ProviderCapabilities};
use crate::thread::TurnStatus;

/// One ended turn of a thread, as a fork needs to know it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRecord {
    pub turn_id: TurnId,
    /// The sequence number of its `turn/completed`.
    pub seq: u64,
    pub status: TurnStatus,
    /// The message that started it, if one did.
    pub prompt: Option<String>,
    /// The agent's last message in it.
    pub reply: Option<String>,
    pub reply_item: Option<ItemId>,
    /// Where its session stood after it, in the provider's terms.
    pub point: Option<String>,
    /// The provider session it ran in, as far as the thread knew then.
    pub session: Option<String>,
    /// The agent it ran with.
    pub provider: ProviderId,
    /// No other turn of the same session started after it.
    pub latest: bool,
}

/// What the running turn has so far: the message that started it, and the agent's
/// latest message in it.
type Running = (Option<String>, Option<(ItemId, String)>);

/// The ended turns of a thread, oldest first.
pub fn turns(events: &[ThreadEvent]) -> Vec<TurnRecord> {
    let mut records: Vec<TurnRecord> = Vec::new();
    let mut provider = ProviderId::new("");
    let mut session: Option<String> = None;
    // The message that will start the next turn, and what the running turn has.
    let mut waiting: Option<String> = None;
    let mut running: Option<Running> = None;
    // The session each turn started in, to tell which turn was a session's latest.
    let mut started_in: Vec<Option<String>> = Vec::new();
    // For each record, which of `started_in` its turn is.
    let mut starts: Vec<usize> = Vec::new();
    for event in events {
        match &event.payload {
            EventPayload::ThreadCreated { thread } => {
                provider = thread.provider.clone();
                session.clone_from(&thread.provider_session);
            }
            EventPayload::SessionStarted { provider_session: Some(name), .. }
            | EventPayload::SessionIdentified { provider_session: name } => {
                session = Some(name.clone());
            }
            EventPayload::AgentChanged { provider: next, .. } => {
                if *next != provider {
                    provider = next.clone();
                    session = None;
                }
            }
            EventPayload::ConversationReset | EventPayload::EnvironmentAttached { .. } => {
                session = None;
            }
            EventPayload::InputSubmitted { text, steered: false, .. } => {
                waiting = Some(text.clone());
            }
            EventPayload::TurnStarted { .. } => {
                running = Some((waiting.take(), None));
                started_in.push(session.clone());
            }
            EventPayload::ItemCompleted { item, .. } => {
                if let (Some((_, reply)), ItemShape::AgentMessage { text }) =
                    (&mut running, &item.shape)
                {
                    *reply = Some((item.id.clone(), text.clone()));
                }
            }
            EventPayload::TurnCompleted { turn_id, status, point, .. } => {
                let (prompt, reply) = running.take().unwrap_or_default();
                // A turn the journal saw complete without starting counts as started now.
                if started_in.len() == starts.len() {
                    started_in.push(session.clone());
                }
                starts.push(started_in.len() - 1);
                let (reply_item, reply) = reply.unzip();
                records.push(TurnRecord {
                    turn_id: turn_id.clone(),
                    seq: event.seq,
                    status: *status,
                    prompt,
                    reply,
                    reply_item,
                    point: point.clone(),
                    session: session.clone(),
                    provider: provider.clone(),
                    latest: true,
                });
            }
            _ => {}
        }
    }
    // A turn is its session's latest when no turn started in that session after it
    // (the one running now included).
    for (record, start) in records.iter_mut().zip(&starts) {
        record.latest = !started_in[start + 1..].contains(&record.session);
    }
    records
}

/// Why a thread cannot be forked from a turn; `None` when it can. `capabilities` are
/// the turn's agent's, with `name` how it is called; absent when no plugin provides
/// it now.
pub fn fork_blocker(
    record: &TurnRecord,
    name: &str,
    capabilities: Option<&ProviderCapabilities>,
) -> Option<String> {
    let Some(capabilities) = capabilities else {
        return Some(format!("No plugin provides {} now", record.provider));
    };
    if capabilities.fork == ForkSupport::None || !capabilities.resume {
        return Some(format!("{name} cannot fork a conversation"));
    }
    if record.point.is_none() || record.session.is_none() {
        return Some(
            "This reply has no point to fork from: it came before qcc could fork, or the agent \
             did not say where its session stood"
                .to_owned(),
        );
    }
    if capabilities.fork == ForkSupport::Latest && !record.latest {
        return Some(format!("{name} forks a conversation only from its latest reply"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ThreadId;
    use crate::item::{Item, ItemStatus};
    use crate::thread::Thread;

    struct Journal(Vec<ThreadEvent>);

    impl Journal {
        fn push(&mut self, payload: EventPayload) -> &mut Self {
            let seq = self.0.len() as u64 + 1;
            self.0.push(ThreadEvent { thread_id: ThreadId::new("t"), seq, ts: seq, payload });
            self
        }

        fn turn(&mut self, n: u32, prompt: &str, point: Option<&str>) -> &mut Self {
            let turn_id = TurnId::new(format!("u{n}"));
            let item = Item {
                id: ItemId::new(format!("i{n}")),
                status: ItemStatus::Completed,
                shape: ItemShape::AgentMessage { text: format!("re {prompt}") },
            };
            self.push(EventPayload::InputSubmitted {
                text: prompt.into(),
                notes: Vec::new(),
                attachments: Vec::new(),
                steered: false,
                queue_id: None,
            })
            .push(EventPayload::TurnStarted { turn_id: turn_id.clone() })
            .push(EventPayload::ItemCompleted { turn_id: turn_id.clone(), item })
            .push(EventPayload::TurnCompleted {
                turn_id,
                status: TurnStatus::Completed,
                error: None,
                point: point.map(str::to_owned),
            })
        }
    }

    fn journal() -> Journal {
        let thread: Thread = serde_json::from_value(serde_json::json!({
            "id": "t", "provider": "echo", "cwd": "/w", "status": "idle",
            "created_at": 0, "updated_at": 0, "last_seq": 0,
        }))
        .unwrap();
        let mut journal = Journal(Vec::new());
        journal.push(EventPayload::ThreadCreated { thread: Box::new(thread) });
        journal
    }

    fn capabilities(fork: ForkSupport) -> ProviderCapabilities {
        ProviderCapabilities { resume: true, fork, ..ProviderCapabilities::default() }
    }

    #[test]
    fn each_turn_knows_its_prompt_reply_point_and_session() {
        let mut journal = journal();
        journal
            .push(EventPayload::SessionStarted {
                provider_session: Some("s1".into()),
                resumed: false,
            })
            .turn(1, "one", Some("p1"))
            .turn(2, "two", Some("p2"))
            .push(EventPayload::ConversationReset)
            .push(EventPayload::SessionStarted {
                provider_session: Some("s2".into()),
                resumed: false,
            })
            .turn(3, "three", None);
        let turns = turns(&journal.0);
        let summary: Vec<_> = turns
            .iter()
            .map(|t| {
                (
                    t.prompt.as_deref(),
                    t.reply.as_deref(),
                    t.point.as_deref(),
                    t.session.as_deref(),
                    t.latest,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (Some("one"), Some("re one"), Some("p1"), Some("s1"), false),
                (Some("two"), Some("re two"), Some("p2"), Some("s1"), true),
                (Some("three"), Some("re three"), None, Some("s2"), true),
            ]
        );
        assert_eq!(turns[0].reply_item, Some(ItemId::new("i1")));
        assert_eq!(turns[1].seq, 10);
    }

    #[test]
    fn a_fork_is_refused_with_a_reason() {
        let mut journal = journal();
        journal
            .push(EventPayload::SessionStarted {
                provider_session: Some("s1".into()),
                resumed: false,
            })
            .turn(1, "one", Some("p1"))
            .turn(2, "two", Some("p2"))
            .turn(3, "three", None);
        let turns = turns(&journal.0);
        let any = capabilities(ForkSupport::Any);
        let latest = capabilities(ForkSupport::Latest);
        assert_eq!(fork_blocker(&turns[0], "Echo", Some(&any)), None);
        assert!(fork_blocker(&turns[2], "Echo", Some(&any)).unwrap().contains("no point"));
        assert!(fork_blocker(&turns[0], "Echo", Some(&latest)).unwrap().contains("latest"));
        assert!(
            fork_blocker(&turns[0], "Echo", Some(&capabilities(ForkSupport::None)))
                .unwrap()
                .contains("cannot fork")
        );
        assert!(fork_blocker(&turns[0], "Echo", None).unwrap().contains("No plugin"));
    }

    #[test]
    fn a_running_turn_makes_the_one_before_it_not_the_latest() {
        let mut journal = journal();
        journal
            .push(EventPayload::SessionStarted {
                provider_session: Some("s1".into()),
                resumed: false,
            })
            .turn(1, "one", Some("p1"))
            .push(EventPayload::TurnStarted { turn_id: TurnId::new("u2") });
        assert!(!turns(&journal.0)[0].latest);
    }
}
