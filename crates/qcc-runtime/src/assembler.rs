use std::collections::HashMap;

use qcc_domain::{
    Delta, EventPayload, InteractionId, InteractionResolution, Item, ItemId, ItemKey, ItemStatus,
    TurnId, TurnStatus,
};

use crate::mint::IdMint;
use crate::timeline::settled_item_status;

/// A delta the grammar does not allow in the current state. The delta is dropped before
/// any state changes; the violation is reported so a broken bridge is visible.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GrammarViolation {
    #[error("turn.open while a turn is open")]
    TurnAlreadyOpen,
    #[error("turn.boundary with no turn open")]
    BoundaryWithoutTurn,
    #[error("{0} with no turn open")]
    NoOpenTurn(&'static str),
    #[error("item.open for {0:?}, which is already open")]
    ItemAlreadyOpen(ItemKey),
    #[error("text stream delta for {0:?}, which is not a text item")]
    NotATextItem(ItemKey),
    #[error("item.close for {0:?} with a pending status")]
    CloseWithPendingStatus(ItemKey),
    #[error("interaction.request {0:?} is already pending")]
    InteractionAlreadyPending(String),
}

struct OpenItem {
    key: ItemKey,
    item: Item,
}

struct PendingInteraction {
    id: InteractionId,
    request_id: String,
}

/// Consumes a bridge's deltas and constructs canonical event payloads.
///
/// Invariants it owns:
///
/// * turn, item and interaction ids are minted here and nowhere else;
/// * only `turn.open` and a claiming `turn.boundary` open a turn;
/// * every item's first event is `item/started` and it settles exactly once;
/// * when a turn settles, nothing stays open underneath it.
pub struct DeltaAssembler {
    mint: Box<dyn IdMint>,
    turn: Option<TurnId>,
    /// Open items in opening order.
    open: Vec<OpenItem>,
    /// Provider-named items that already settled in this provider session: repeated
    /// closes are deduplicated and an explicit reopen reuses the id.
    settled: HashMap<String, ItemId>,
    /// Input was submitted and no turn has opened since.
    input_pending: bool,
    interactions: Vec<PendingInteraction>,
    violations: Vec<GrammarViolation>,
}

impl DeltaAssembler {
    pub fn new(mint: Box<dyn IdMint>) -> Self {
        Self {
            mint,
            turn: None,
            open: Vec::new(),
            settled: HashMap::new(),
            input_pending: false,
            interactions: Vec::new(),
            violations: Vec::new(),
        }
    }

    /// Record that user input was handed to the provider. A later claiming boundary
    /// may own a turn because of it.
    pub fn note_input(&mut self) {
        if self.turn.is_none() {
            self.input_pending = true;
        }
    }

    /// An id for a queued input, from the same source as every other timeline id.
    pub fn mint_queue_id(&mut self) -> String {
        self.mint.queued()
    }

    /// Violations collected since the last call.
    pub fn take_violations(&mut self) -> Vec<GrammarViolation> {
        std::mem::take(&mut self.violations)
    }

    /// Forget a pending interaction and return the provider-side request id the
    /// resolution has to be sent back with.
    pub fn take_interaction(&mut self, id: &InteractionId) -> Option<String> {
        let index = self.interactions.iter().position(|pending| &pending.id == id)?;
        Some(self.interactions.remove(index).request_id)
    }

    pub fn apply(&mut self, delta: Delta) -> Vec<EventPayload> {
        let mut out = Vec::new();
        match delta {
            Delta::TurnOpen => {
                if self.turn.is_some() {
                    self.violations.push(GrammarViolation::TurnAlreadyOpen);
                } else {
                    self.start_turn(&mut out);
                }
            }
            Delta::TurnBoundary { status, error, claim_if_idle } => {
                if self.turn.is_none() {
                    if !claim_if_idle {
                        self.violations.push(GrammarViolation::BoundaryWithoutTurn);
                        return out;
                    }
                    if !self.input_pending {
                        return out;
                    }
                    self.start_turn(&mut out);
                }
                self.settle_into(status, error, &mut out);
            }
            Delta::ItemOpen { key, shape } => {
                let Some(turn_id) = self.require_turn("item.open") else {
                    return out;
                };
                if self.position(&key).is_some() {
                    self.violations.push(GrammarViolation::ItemAlreadyOpen(key));
                    return out;
                }
                if !shape.is_text() {
                    self.release_streams(&turn_id, &mut out);
                }
                let id = self.id_for(&key);
                let item = Item { id, status: ItemStatus::Pending, shape };
                out.push(EventPayload::ItemStarted { turn_id, item: item.clone() });
                self.open.push(OpenItem { key, item });
            }
            Delta::ItemClose { key, shape, status } => {
                let Some(turn_id) = self.require_turn("item.close") else {
                    return out;
                };
                if !status.is_terminal() {
                    self.violations.push(GrammarViolation::CloseWithPendingStatus(key));
                    return out;
                }
                let item = match self.position(&key) {
                    Some(index) => {
                        // The carried shape wins, including a mid-flight reclassification.
                        let open = self.open.remove(index);
                        Item { id: open.item.id, status, shape }
                    }
                    None => {
                        if self.is_settled(&key) {
                            return out;
                        }
                        if !shape.is_text() {
                            self.release_streams(&turn_id, &mut out);
                        }
                        let id = self.id_for(&key);
                        out.push(EventPayload::ItemStarted {
                            turn_id: turn_id.clone(),
                            item: Item {
                                id: id.clone(),
                                status: ItemStatus::Pending,
                                shape: shape.clone(),
                            },
                        });
                        Item { id, status, shape }
                    }
                };
                self.remember_settled(&key, &item.id);
                out.push(EventPayload::ItemCompleted { turn_id, item });
            }
            Delta::ItemTextDelta { key, channel, text } => {
                let Some(turn_id) = self.require_turn("item.text-delta") else {
                    return out;
                };
                let index = match self.position(&key) {
                    Some(index) => index,
                    None => {
                        let id = self.id_for(&key);
                        let item = Item {
                            id,
                            status: ItemStatus::Pending,
                            shape: channel.shape(String::new()),
                        };
                        out.push(EventPayload::ItemStarted { turn_id, item: item.clone() });
                        self.open.push(OpenItem { key: key.clone(), item });
                        self.open.len() - 1
                    }
                };
                let item = &mut self.open[index].item;
                let Some(buffer) = item.shape.text_mut() else {
                    self.violations.push(GrammarViolation::NotATextItem(key));
                    return out;
                };
                if !text.is_empty() {
                    buffer.push_str(&text);
                    out.push(EventPayload::ItemTextDelta { item_id: item.id.clone(), text });
                }
            }
            Delta::ItemTextClose { key, channel, text } => {
                let Some(turn_id) = self.require_turn("item.text-close") else {
                    return out;
                };
                let item = match self.position(&key) {
                    Some(index) => {
                        if !self.open[index].item.shape.is_text() {
                            self.violations.push(GrammarViolation::NotATextItem(key));
                            return out;
                        }
                        let mut item = self.open.remove(index).item;
                        if let (Some(buffer), Some(text)) = (item.shape.text_mut(), text) {
                            *buffer = text;
                        }
                        item.status = ItemStatus::Completed;
                        item
                    }
                    None => {
                        // Nothing was streamed: only a provider-final text makes an item.
                        let Some(text) = text else {
                            return out;
                        };
                        if self.is_settled(&key) {
                            return out;
                        }
                        let id = self.id_for(&key);
                        let shape = channel.shape(text);
                        out.push(EventPayload::ItemStarted {
                            turn_id: turn_id.clone(),
                            item: Item {
                                id: id.clone(),
                                status: ItemStatus::Pending,
                                shape: shape.clone(),
                            },
                        });
                        Item { id, status: ItemStatus::Completed, shape }
                    }
                };
                self.remember_settled(&key, &item.id);
                out.push(EventPayload::ItemCompleted { turn_id, item });
            }
            Delta::Usage { usage } => out.push(EventPayload::UsageUpdated { usage }),
            Delta::ContextUsage { context } => {
                out.push(EventPayload::ContextUsageUpdated { context });
            }
            Delta::ContextCompacted { compaction } => {
                out.push(EventPayload::ContextCompacted { compaction });
            }
            Delta::InteractionRequest { request_id, item, request } => {
                if self.interactions.iter().any(|pending| pending.request_id == request_id) {
                    self.violations.push(GrammarViolation::InteractionAlreadyPending(request_id));
                    return out;
                }
                let interaction_id = self.mint.interaction();
                let item_id = item
                    .and_then(|key| self.position(&key))
                    .map(|index| self.open[index].item.id.clone());
                self.interactions
                    .push(PendingInteraction { id: interaction_id.clone(), request_id });
                out.push(EventPayload::InteractionRequested {
                    interaction_id,
                    turn_id: self.turn.clone(),
                    item_id,
                    request,
                });
            }
            Delta::SessionIdentity { provider_session } => {
                out.push(EventPayload::SessionIdentified { provider_session });
            }
            Delta::SessionReset => {
                self.settle(TurnStatus::Interrupted, None, &mut out);
                self.settled.clear();
            }
            Delta::SessionEnded { reason } => {
                self.settle(TurnStatus::Interrupted, None, &mut out);
                self.settled.clear();
                out.push(EventPayload::SessionEnded { reason });
            }
            Delta::ProviderError { message } => out.push(EventPayload::ProviderError { message }),
            Delta::ProviderWarning { message } => {
                out.push(EventPayload::ProviderWarning { message });
            }
            Delta::Unhandled { payload } => out.push(EventPayload::ProviderUnhandled { payload }),
            // The engine keeps the MCP servers: they belong to the thread, not a turn.
            Delta::McpStatus { .. } | Delta::McpRemoved { .. } => {}
        }
        out
    }

    /// Close everything that is open: pending interactions are cancelled; open items
    /// and the open turn, if any, settle with `status`.
    pub fn settle(
        &mut self,
        status: TurnStatus,
        error: Option<String>,
        out: &mut Vec<EventPayload>,
    ) {
        if self.turn.is_some() {
            self.settle_into(status, error, out);
        } else {
            self.cancel_interactions(out);
        }
    }

    fn start_turn(&mut self, out: &mut Vec<EventPayload>) {
        let turn_id = self.mint.turn();
        self.turn = Some(turn_id.clone());
        self.input_pending = false;
        out.push(EventPayload::TurnStarted { turn_id });
    }

    fn settle_into(
        &mut self,
        status: TurnStatus,
        error: Option<String>,
        out: &mut Vec<EventPayload>,
    ) {
        let Some(turn_id) = self.turn.take() else {
            return;
        };
        self.cancel_interactions(out);
        for open in std::mem::take(&mut self.open) {
            let mut item = open.item;
            item.status = settled_item_status(&item, status);
            self.remember_settled(&open.key, &item.id);
            out.push(EventPayload::ItemCompleted { turn_id: turn_id.clone(), item });
        }
        self.input_pending = false;
        out.push(EventPayload::TurnCompleted { turn_id, status, error });
    }

    fn cancel_interactions(&mut self, out: &mut Vec<EventPayload>) {
        for pending in std::mem::take(&mut self.interactions) {
            out.push(EventPayload::InteractionResolved {
                interaction_id: pending.id,
                resolution: InteractionResolution::Cancelled,
            });
        }
    }

    /// A non-text item ends the anonymous text streams before it: later text mints a
    /// fresh item instead of continuing above the tool call.
    fn release_streams(&mut self, turn_id: &TurnId, out: &mut Vec<EventPayload>) {
        let (streams, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut self.open)
            .into_iter()
            .partition(|open| matches!(open.key, ItemKey::Stream(_)) && open.item.shape.is_text());
        self.open = rest;
        for open in streams {
            let mut item = open.item;
            item.status = ItemStatus::Completed;
            out.push(EventPayload::ItemCompleted { turn_id: turn_id.clone(), item });
        }
    }

    fn require_turn(&mut self, delta: &'static str) -> Option<TurnId> {
        if self.turn.is_none() {
            self.violations.push(GrammarViolation::NoOpenTurn(delta));
        }
        self.turn.clone()
    }

    fn position(&self, key: &ItemKey) -> Option<usize> {
        self.open.iter().position(|open| &open.key == key)
    }

    fn is_settled(&self, key: &ItemKey) -> bool {
        matches!(key, ItemKey::Provider(id) if self.settled.contains_key(id))
    }

    /// A provider-named item that settled earlier keeps its id when it reopens.
    fn id_for(&mut self, key: &ItemKey) -> ItemId {
        if let ItemKey::Provider(id) = key
            && let Some(item_id) = self.settled.remove(id)
        {
            return item_id;
        }
        self.mint.item()
    }

    fn remember_settled(&mut self, key: &ItemKey, item_id: &ItemId) {
        if let ItemKey::Provider(id) = key {
            self.settled.insert(id.clone(), item_id.clone());
        }
    }
}
