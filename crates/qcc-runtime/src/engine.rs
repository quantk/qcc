use qcc_domain::{
    Delta, EventPayload, InteractionId, InteractionResolution, Thread, ThreadEvent, ThreadStatus,
    TurnStatus,
};

use crate::assembler::{DeltaAssembler, GrammarViolation};
use crate::mint::IdMint;
use crate::timeline::Timeline;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("interaction {0} is not pending")]
    InteractionNotPending(InteractionId),
}

/// One thread: its projection, its assembler and its sequence counter.
///
/// Every method that changes the thread returns the events it produced, already
/// sequenced and folded into [`ThreadEngine::thread`]. The caller persists them.
pub struct ThreadEngine {
    thread: Thread,
    assembler: DeltaAssembler,
    timeline: Timeline,
    session_live: bool,
    failed: bool,
}

impl ThreadEngine {
    /// Start a new thread. `thread` is the initial projection (`last_seq` 0).
    pub fn create(mut thread: Thread, mint: Box<dyn IdMint>, now: u64) -> (Self, Vec<ThreadEvent>) {
        thread.status = ThreadStatus::Idle;
        thread.last_seq = 0;
        thread.created_at = now;
        thread.updated_at = now;
        let created = EventPayload::ThreadCreated {
            thread: thread.clone(),
        };
        let mut engine = Self {
            thread,
            assembler: DeltaAssembler::new(mint),
            timeline: Timeline::default(),
            session_live: false,
            failed: false,
        };
        let events = engine.emit(vec![created], now);
        (engine, events)
    }

    /// Rebuild an engine from a persisted thread after a restart.
    ///
    /// No provider session survives a restart, so whatever `history` left open is
    /// settled as interrupted. `history` may be empty when the thread is not active.
    pub fn recover<'a>(
        thread: Thread,
        history: impl IntoIterator<Item = &'a EventPayload>,
        mint: Box<dyn IdMint>,
        now: u64,
    ) -> (Self, Vec<ThreadEvent>) {
        let mut timeline = Timeline::default();
        for payload in history {
            // A journal written by an older build may violate today's grammar; the
            // fold is still the best description of what is open.
            let _ = timeline.apply(payload);
        }
        let failed = thread.status == ThreadStatus::Failed;
        let mut engine = Self {
            thread,
            assembler: DeltaAssembler::new(mint),
            timeline,
            session_live: false,
            failed,
        };
        let settlement = engine.timeline.settlement(TurnStatus::Interrupted, None);
        let events = engine.emit(settlement, now);
        (engine, events)
    }

    pub fn thread(&self) -> &Thread {
        &self.thread
    }

    /// Whether a provider session is attached. Input for a thread without one has to
    /// start (or resume) a session first.
    pub fn session_live(&self) -> bool {
        self.session_live
    }

    pub fn take_violations(&mut self) -> Vec<GrammarViolation> {
        self.assembler.take_violations()
    }

    pub fn submit_input(&mut self, text: String, now: u64) -> Vec<ThreadEvent> {
        self.assembler.note_input();
        self.emit(vec![EventPayload::InputSubmitted { text }], now)
    }

    pub fn session_started(
        &mut self,
        provider_session: Option<String>,
        resumed: bool,
        now: u64,
    ) -> Vec<ThreadEvent> {
        self.session_live = true;
        self.failed = false;
        self.emit(
            vec![EventPayload::SessionStarted {
                provider_session,
                resumed,
            }],
            now,
        )
    }

    /// The provider session could not start or died: record why, settle open work as
    /// failed and mark the thread failed until a session starts again.
    pub fn session_failed(&mut self, message: String, now: u64) -> Vec<ThreadEvent> {
        self.session_live = false;
        self.failed = true;
        let mut payloads = vec![EventPayload::ProviderError {
            message: message.clone(),
        }];
        self.assembler
            .settle(TurnStatus::Failed, Some(message), &mut payloads);
        self.emit(payloads, now)
    }

    pub fn apply_deltas(
        &mut self,
        deltas: impl IntoIterator<Item = Delta>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        let mut payloads = Vec::new();
        for delta in deltas {
            payloads.extend(self.assembler.apply(delta));
        }
        if payloads
            .iter()
            .any(|p| matches!(p, EventPayload::SessionEnded { .. }))
        {
            self.session_live = false;
        }
        self.emit(payloads, now)
    }

    /// Record the user's answer. Returns the provider-side request id the resolution
    /// has to be delivered with.
    pub fn resolve_interaction(
        &mut self,
        interaction_id: InteractionId,
        resolution: InteractionResolution,
        now: u64,
    ) -> Result<(String, Vec<ThreadEvent>), EngineError> {
        let request_id = self
            .assembler
            .take_interaction(&interaction_id)
            .ok_or_else(|| EngineError::InteractionNotPending(interaction_id.clone()))?;
        let events = self.emit(
            vec![EventPayload::InteractionResolved {
                interaction_id,
                resolution,
            }],
            now,
        );
        Ok((request_id, events))
    }

    /// Settle whatever is still open as interrupted. The backstop after a provider was
    /// asked to interrupt: a bridge that already settled its turn makes this a no-op.
    pub fn force_settle(&mut self, now: u64) -> Vec<ThreadEvent> {
        let mut payloads = Vec::new();
        self.assembler
            .settle(TurnStatus::Interrupted, None, &mut payloads);
        self.emit(payloads, now)
    }

    /// The provider session was released without an error.
    pub fn session_released(&mut self, reason: Option<String>, now: u64) -> Vec<ThreadEvent> {
        if !self.session_live {
            return Vec::new();
        }
        self.apply_deltas([Delta::SessionEnded { reason }], now)
    }

    fn emit(&mut self, payloads: Vec<EventPayload>, now: u64) -> Vec<ThreadEvent> {
        let mut events = Vec::with_capacity(payloads.len() + 1);
        for payload in payloads {
            let applied = self.timeline.apply(&payload);
            debug_assert!(
                applied.is_ok(),
                "assembler broke the timeline grammar: {applied:?}"
            );
            self.push(payload, now, &mut events);
        }
        let status = if self.timeline.open_turn().is_some() {
            ThreadStatus::Active
        } else if self.failed {
            ThreadStatus::Failed
        } else {
            ThreadStatus::Idle
        };
        if status != self.thread.status {
            self.push(
                EventPayload::ThreadStatusChanged { status },
                now,
                &mut events,
            );
        }
        events
    }

    fn push(&mut self, payload: EventPayload, now: u64, events: &mut Vec<ThreadEvent>) {
        let event = ThreadEvent {
            thread_id: self.thread.id.clone(),
            seq: self.thread.last_seq + 1,
            ts: now,
            payload,
        };
        self.thread.apply(&event);
        events.push(event);
    }
}
