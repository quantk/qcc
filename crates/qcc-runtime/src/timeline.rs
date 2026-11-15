use std::collections::{BTreeMap, HashSet};

use qcc_domain::{
    EventPayload, InteractionId, InteractionResolution, Item, ItemId, ItemStatus, TurnId,
    TurnStatus,
};

/// A canonical timeline that breaks its own grammar.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimelineViolation {
    #[error("turn {0} started while turn {1} is open")]
    TurnWhileTurnOpen(TurnId, TurnId),
    #[error("turn id {0} was used before")]
    TurnIdReused(TurnId),
    #[error("turn {0} completed but it is not the open turn")]
    TurnNotOpen(TurnId),
    #[error("turn {0} completed with item {1} still open")]
    TurnCompletedWithOpenItem(TurnId, ItemId),
    #[error("turn {0} completed with interaction {1} still pending")]
    TurnCompletedWithPendingInteraction(TurnId, InteractionId),
    #[error("item {0} refers to turn {1}, which is not the open turn")]
    ItemOutsideTurn(ItemId, TurnId),
    #[error("item {0} started while it is already open")]
    ItemAlreadyOpen(ItemId),
    #[error("item {0} started in a non-pending state")]
    ItemStartedSettled(ItemId),
    #[error("event for item {0}, which no item/started opened")]
    ItemNotOpen(ItemId),
    #[error("item {0} completed in a pending state")]
    ItemCompletedPending(ItemId),
    #[error("interaction {0} requested twice")]
    InteractionRequestedTwice(InteractionId),
    #[error("interaction {0} resolved but it is not pending")]
    InteractionNotPending(InteractionId),
    #[error("queued input {0} delivered or dropped but it is not queued")]
    InputNotQueued(String),
}

/// What is open on a thread right now: the fold of its events.
///
/// The assembler's output always satisfies this grammar; the checker exists so tests,
/// the conformance kit and crash recovery can rely on the same rules.
#[derive(Debug, Default, Clone)]
pub struct Timeline {
    open_turn: Option<TurnId>,
    /// Open items in opening order.
    open_items: BTreeMap<u64, Item>,
    next_item_ordinal: u64,
    pending_interactions: Vec<InteractionId>,
    seen_turns: HashSet<TurnId>,
    /// Inputs waiting for the running turn to end: `(queue id, text)`, oldest first.
    queued: Vec<(String, String)>,
}

impl Timeline {
    pub fn open_turn(&self) -> Option<&TurnId> {
        self.open_turn.as_ref()
    }

    pub fn open_items(&self) -> impl Iterator<Item = &Item> {
        self.open_items.values()
    }

    pub fn pending_interactions(&self) -> &[InteractionId] {
        &self.pending_interactions
    }

    /// Inputs waiting for delivery: `(queue id, text)`, oldest first.
    pub fn queued_inputs(&self) -> &[(String, String)] {
        &self.queued
    }

    /// Apply one event. A violating event is still applied on a best-effort basis so
    /// that the fold stays usable for recovery.
    pub fn apply(&mut self, payload: &EventPayload) -> Result<(), TimelineViolation> {
        match payload {
            EventPayload::TurnStarted { turn_id } => {
                let fresh = self.seen_turns.insert(turn_id.clone());
                let previous = self.open_turn.replace(turn_id.clone());
                if let Some(previous) = previous {
                    return Err(TimelineViolation::TurnWhileTurnOpen(turn_id.clone(), previous));
                }
                if !fresh {
                    return Err(TimelineViolation::TurnIdReused(turn_id.clone()));
                }
                Ok(())
            }
            EventPayload::TurnCompleted { turn_id, .. } => {
                if self.open_turn.as_ref() != Some(turn_id) {
                    return Err(TimelineViolation::TurnNotOpen(turn_id.clone()));
                }
                self.open_turn = None;
                let open_item = self.open_items.values().next().map(|item| item.id.clone());
                let pending = self.pending_interactions.first().cloned();
                self.open_items.clear();
                self.pending_interactions.clear();
                if let Some(item_id) = open_item {
                    return Err(TimelineViolation::TurnCompletedWithOpenItem(
                        turn_id.clone(),
                        item_id,
                    ));
                }
                if let Some(interaction_id) = pending {
                    return Err(TimelineViolation::TurnCompletedWithPendingInteraction(
                        turn_id.clone(),
                        interaction_id,
                    ));
                }
                Ok(())
            }
            EventPayload::ItemStarted { turn_id, item } => {
                if self.open_turn.as_ref() != Some(turn_id) {
                    return Err(TimelineViolation::ItemOutsideTurn(
                        item.id.clone(),
                        turn_id.clone(),
                    ));
                }
                if self.ordinal_of(&item.id).is_some() {
                    return Err(TimelineViolation::ItemAlreadyOpen(item.id.clone()));
                }
                self.open_items.insert(self.next_item_ordinal, item.clone());
                self.next_item_ordinal += 1;
                if item.status != ItemStatus::Pending {
                    return Err(TimelineViolation::ItemStartedSettled(item.id.clone()));
                }
                Ok(())
            }
            EventPayload::ItemTextDelta { item_id, text } => {
                let ordinal = self
                    .ordinal_of(item_id)
                    .ok_or_else(|| TimelineViolation::ItemNotOpen(item_id.clone()))?;
                if let Some(item) = self.open_items.get_mut(&ordinal)
                    && let Some(buffer) = item.shape.text_mut()
                {
                    buffer.push_str(text);
                }
                Ok(())
            }
            EventPayload::ItemCompleted { turn_id, item } => {
                let ordinal = self
                    .ordinal_of(&item.id)
                    .ok_or_else(|| TimelineViolation::ItemNotOpen(item.id.clone()))?;
                self.open_items.remove(&ordinal);
                if self.open_turn.as_ref() != Some(turn_id) {
                    return Err(TimelineViolation::ItemOutsideTurn(
                        item.id.clone(),
                        turn_id.clone(),
                    ));
                }
                if item.status == ItemStatus::Pending {
                    return Err(TimelineViolation::ItemCompletedPending(item.id.clone()));
                }
                Ok(())
            }
            EventPayload::InteractionRequested { interaction_id, .. } => {
                if self.pending_interactions.contains(interaction_id) {
                    return Err(TimelineViolation::InteractionRequestedTwice(
                        interaction_id.clone(),
                    ));
                }
                self.pending_interactions.push(interaction_id.clone());
                Ok(())
            }
            EventPayload::InteractionResolved { interaction_id, .. } => {
                let before = self.pending_interactions.len();
                self.pending_interactions.retain(|id| id != interaction_id);
                if self.pending_interactions.len() == before {
                    return Err(TimelineViolation::InteractionNotPending(interaction_id.clone()));
                }
                Ok(())
            }
            EventPayload::InputQueued { queue_id, text } => {
                self.queued.push((queue_id.clone(), text.clone()));
                Ok(())
            }
            EventPayload::InputSubmitted { queue_id: Some(queue_id), .. }
            | EventPayload::InputDropped { queue_id } => {
                let before = self.queued.len();
                self.queued.retain(|(id, _)| id != queue_id);
                if self.queued.len() == before {
                    return Err(TimelineViolation::InputNotQueued(queue_id.clone()));
                }
                Ok(())
            }
            EventPayload::ThreadCreated { .. }
            | EventPayload::ThreadArchived
            | EventPayload::EnvironmentPreparing
            | EventPayload::EnvironmentReady { .. }
            | EventPayload::EnvironmentFailed { .. }
            | EventPayload::EnvironmentRemoved
            | EventPayload::PermissionModeChanged { .. }
            | EventPayload::AgentChanged { .. }
            | EventPayload::ProjectChanged { .. }
            | EventPayload::TitleChanged { .. }
            | EventPayload::ThreadStatusChanged { .. }
            | EventPayload::SessionStarted { .. }
            | EventPayload::SessionIdentified { .. }
            | EventPayload::SessionEnded { .. }
            | EventPayload::InputSubmitted { .. }
            | EventPayload::UsageUpdated { .. }
            | EventPayload::ProviderError { .. }
            | EventPayload::ProviderWarning { .. }
            | EventPayload::ProviderUnhandled { .. } => Ok(()),
        }
    }

    /// Check a whole timeline; returns every violation with the index of its event.
    pub fn check<'a>(
        payloads: impl IntoIterator<Item = &'a EventPayload>,
    ) -> Vec<(usize, TimelineViolation)> {
        let mut timeline = Self::default();
        payloads
            .into_iter()
            .enumerate()
            .filter_map(|(index, payload)| timeline.apply(payload).err().map(|v| (index, v)))
            .collect()
    }

    /// Events that close everything still open: pending interactions are cancelled,
    /// open items and the open turn are settled with `status`.
    ///
    /// Used when the owner of the open work is gone (daemon restart, dead provider).
    pub fn settlement(&self, status: TurnStatus, error: Option<String>) -> Vec<EventPayload> {
        let mut events = Vec::new();
        for interaction_id in &self.pending_interactions {
            events.push(EventPayload::InteractionResolved {
                interaction_id: interaction_id.clone(),
                resolution: InteractionResolution::Cancelled,
            });
        }
        let Some(turn_id) = self.open_turn.clone() else {
            return events;
        };
        for item in self.open_items.values() {
            let mut item = item.clone();
            item.status = settled_item_status(&item, status);
            events.push(EventPayload::ItemCompleted { turn_id: turn_id.clone(), item });
        }
        events.push(EventPayload::TurnCompleted { turn_id, status, error });
        events
    }

    fn ordinal_of(&self, item_id: &ItemId) -> Option<u64> {
        self.open_items.iter().find(|(_, item)| &item.id == item_id).map(|(ordinal, _)| *ordinal)
    }
}

/// Status of an item its bridge left open when the turn settled. Streamed text keeps
/// what it received; anything else did not finish unless the turn itself completed.
pub(crate) fn settled_item_status(item: &Item, turn: TurnStatus) -> ItemStatus {
    if item.shape.is_text() {
        return ItemStatus::Completed;
    }
    match turn {
        TurnStatus::Completed => ItemStatus::Completed,
        TurnStatus::Failed => ItemStatus::Failed,
        TurnStatus::Interrupted => ItemStatus::Interrupted,
    }
}
