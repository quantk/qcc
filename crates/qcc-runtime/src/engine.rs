use qcc_domain::{
    Delta, EventPayload, InteractionId, InteractionResolution, PermissionMode, ProjectId,
    ProviderId, Thread, ThreadEvent, ThreadStatus, TurnStatus, UserInput,
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
    /// Input was handed to the provider and its turn has not opened yet.
    awaiting_turn: bool,
}

impl ThreadEngine {
    /// Start a new thread. `thread` is the initial projection (`last_seq` 0).
    pub fn create(mut thread: Thread, mint: Box<dyn IdMint>, now: u64) -> (Self, Vec<ThreadEvent>) {
        thread.status = ThreadStatus::Idle;
        thread.last_seq = 0;
        thread.created_at = now;
        thread.updated_at = now;
        let created = EventPayload::ThreadCreated { thread: Box::new(thread.clone()) };
        let mut engine = Self {
            thread,
            assembler: DeltaAssembler::new(mint),
            timeline: Timeline::default(),
            session_live: false,
            failed: false,
            awaiting_turn: false,
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
            awaiting_turn: false,
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

    /// Whether a turn is running or about to: input was handed to the provider and
    /// its turn has not opened yet. New input has to be steered or queued.
    pub fn busy(&self) -> bool {
        self.turn_open() || self.awaiting_turn
    }

    pub fn turn_open(&self) -> bool {
        self.timeline.open_turn().is_some()
    }

    pub fn has_queued_input(&self) -> bool {
        !self.timeline.queued_inputs().is_empty()
    }

    /// A user message that starts a turn. The caller delivers it to the provider.
    pub fn submit_input(&mut self, input: UserInput, now: u64) -> Vec<ThreadEvent> {
        self.assembler.note_input();
        self.awaiting_turn = !self.turn_open();
        let UserInput { text, notes } = input;
        let submitted =
            EventPayload::InputSubmitted { text, notes, steered: false, queue_id: None };
        self.emit(vec![submitted], now)
    }

    /// A user message fed into the running turn.
    pub fn steer_input(&mut self, input: UserInput, now: u64) -> Vec<ThreadEvent> {
        let UserInput { text, notes } = input;
        let submitted = EventPayload::InputSubmitted { text, notes, steered: true, queue_id: None };
        self.emit(vec![submitted], now)
    }

    /// A user message that waits for the running turn to end.
    pub fn queue_input(&mut self, input: UserInput, now: u64) -> Vec<ThreadEvent> {
        let queue_id = self.assembler.mint_queue_id();
        let UserInput { text, notes } = input;
        self.emit(vec![EventPayload::InputQueued { queue_id, text, notes }], now)
    }

    /// Take the oldest queued message off the queue if the thread can start a turn.
    /// Returns it for the caller to deliver, with `notes` added to its own: what
    /// qcc has to tell the agent now. One message per turn: the next one waits
    /// until this one's turn has come and gone.
    pub fn deliver_queued(
        &mut self,
        notes: &[String],
        now: u64,
    ) -> Option<(UserInput, Vec<ThreadEvent>)> {
        if self.busy() {
            return None;
        }
        let (queue_id, mut input) = self.timeline.queued_inputs().first().cloned()?;
        input.notes.extend_from_slice(notes);
        self.assembler.note_input();
        self.awaiting_turn = true;
        let submitted = EventPayload::InputSubmitted {
            text: input.text.clone(),
            notes: input.notes.clone(),
            steered: false,
            queue_id: Some(queue_id),
        };
        Some((input, self.emit(vec![submitted], now)))
    }

    /// Discard everything queued: the user stopped the thread.
    pub fn drop_queued(&mut self, now: u64) -> Vec<ThreadEvent> {
        let dropped = self
            .timeline
            .queued_inputs()
            .iter()
            .map(|(queue_id, _)| EventPayload::InputDropped { queue_id: queue_id.clone() })
            .collect();
        self.emit(dropped, now)
    }

    /// Take one message off the queue before it is delivered. Returns its text, or
    /// `None` when nothing with that id is queued (it may have been delivered).
    pub fn unqueue(&mut self, queue_id: &str, now: u64) -> Option<(String, Vec<ThreadEvent>)> {
        let (_, input) =
            self.timeline.queued_inputs().iter().find(|(id, _)| id == queue_id)?.clone();
        let dropped = EventPayload::InputDropped { queue_id: queue_id.to_owned() };
        Some((input.text, self.emit(vec![dropped], now)))
    }

    /// The provider was asked to compact the session's context, which it does in a
    /// turn of its own: until that turn has come and gone, input waits.
    pub fn compact_requested(&mut self) {
        self.assembler.note_input();
        self.awaiting_turn = !self.turn_open();
    }

    /// The provider refused the submitted input: no turn will open for it.
    pub fn input_rejected(&mut self) {
        self.awaiting_turn = false;
    }

    pub fn set_permission_mode(
        &mut self,
        permission_mode: Option<PermissionMode>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        if self.thread.permission_mode == permission_mode {
            return Vec::new();
        }
        self.emit(vec![EventPayload::PermissionModeChanged { permission_mode }], now)
    }

    /// Go on with another agent, model or effort. The caller releases a live session
    /// first: all three are fixed when a session is constructed.
    pub fn set_agent(
        &mut self,
        provider: ProviderId,
        model: Option<String>,
        effort: Option<String>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        let thread = &self.thread;
        if thread.provider == provider && thread.model == model && thread.effort == effort {
            return Vec::new();
        }
        self.emit(vec![EventPayload::AgentChanged { provider, model, effort }], now)
    }

    pub fn set_title(&mut self, title: String, now: u64) -> Vec<ThreadEvent> {
        if self.thread.title.as_deref() == Some(title.as_str()) {
            return Vec::new();
        }
        self.emit(vec![EventPayload::TitleChanged { title }], now)
    }

    pub fn set_project(&mut self, project: Option<ProjectId>, now: u64) -> Vec<ThreadEvent> {
        if self.thread.project == project {
            return Vec::new();
        }
        self.emit(vec![EventPayload::ProjectChanged { project }], now)
    }

    pub fn set_parent(
        &mut self,
        parent: Option<qcc_domain::ThreadId>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        if self.thread.parent == parent {
            return Vec::new();
        }
        self.emit(vec![EventPayload::ParentChanged { parent }], now)
    }

    /// Record a step in the life of the thread's environment or of the thread
    /// itself (`environment/*`, `thread/archived`, `thread/reset`). These carry no
    /// timeline grammar.
    pub fn record(&mut self, payload: EventPayload, now: u64) -> Vec<ThreadEvent> {
        self.emit(vec![payload], now)
    }

    pub fn session_started(
        &mut self,
        provider_session: Option<String>,
        resumed: bool,
        now: u64,
    ) -> Vec<ThreadEvent> {
        self.session_live = true;
        self.failed = false;
        self.emit(vec![EventPayload::SessionStarted { provider_session, resumed }], now)
    }

    /// The provider session could not start or died: record why, settle open work as
    /// failed and mark the thread failed until a session starts again.
    pub fn session_failed(&mut self, message: String, now: u64) -> Vec<ThreadEvent> {
        self.session_live = false;
        self.failed = true;
        self.awaiting_turn = false;
        let mut payloads = vec![EventPayload::ProviderError { message: message.clone() }];
        self.assembler.settle(TurnStatus::Failed, Some(message), &mut payloads);
        self.emit(payloads, now)
    }

    pub fn apply_deltas(
        &mut self,
        deltas: impl IntoIterator<Item = Delta>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        let mut payloads = Vec::new();
        // What the batch has made of the MCP servers so far; reported whole.
        let mut mcp = self.thread.mcp_servers.clone();
        for delta in deltas {
            match delta {
                Delta::McpStatus { server } => {
                    match mcp.iter_mut().find(|known| known.name == server.name) {
                        Some(known) if *known == server => continue,
                        Some(known) => *known = server,
                        None => mcp.push(server),
                    }
                    payloads.push(EventPayload::McpServersChanged { servers: mcp.clone() });
                }
                Delta::SessionReset => {
                    // The servers of the old session say nothing about the new one.
                    if !mcp.is_empty() {
                        mcp.clear();
                        payloads.push(EventPayload::McpServersChanged { servers: Vec::new() });
                    }
                    payloads.extend(self.assembler.apply(Delta::SessionReset));
                }
                delta => payloads.extend(self.assembler.apply(delta)),
            }
        }
        if payloads.iter().any(|p| matches!(p, EventPayload::SessionEnded { .. })) {
            self.session_live = false;
            self.awaiting_turn = false;
        }
        self.emit(payloads, now)
    }

    /// Keep these MCP servers off in the thread's sessions from now on.
    pub fn set_mcp_disabled(&mut self, disabled: Vec<String>, now: u64) -> Vec<ThreadEvent> {
        let disabled = sorted(disabled);
        if disabled == self.thread.mcp_disabled {
            return Vec::new();
        }
        self.emit(vec![EventPayload::McpDisabledChanged { disabled }], now)
    }

    /// The skills the thread takes beyond, and leaves out of, what every session gets.
    pub fn set_skills(
        &mut self,
        enabled: Vec<String>,
        disabled: Vec<String>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        let (enabled, disabled) = (sorted(enabled), sorted(disabled));
        if enabled == self.thread.skills_enabled && disabled == self.thread.skills_disabled {
            return Vec::new();
        }
        self.emit(vec![EventPayload::SkillsChanged { enabled, disabled }], now)
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
        let events =
            self.emit(vec![EventPayload::InteractionResolved { interaction_id, resolution }], now);
        Ok((request_id, events))
    }

    /// Settle whatever is still open as interrupted. The backstop after a provider was
    /// asked to interrupt: a bridge that already settled its turn makes this a no-op.
    pub fn force_settle(&mut self, now: u64) -> Vec<ThreadEvent> {
        let mut payloads = Vec::new();
        self.assembler.settle(TurnStatus::Interrupted, None, &mut payloads);
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
            debug_assert!(applied.is_ok(), "assembler broke the timeline grammar: {applied:?}");
            if matches!(payload, EventPayload::TurnStarted { .. }) {
                self.awaiting_turn = false;
            }
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
            self.push(EventPayload::ThreadStatusChanged { status }, now, &mut events);
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

/// Names sorted and without repeats, as the thread keeps its lists.
pub(crate) fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names.dedup();
    names
}
