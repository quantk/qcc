use qcc_domain::{InteractionId, ItemId, TurnId};

/// Source of the ids the assembler stamps on the timeline.
pub trait IdMint: Send {
    fn turn(&mut self) -> TurnId;
    fn item(&mut self) -> ItemId;
    fn interaction(&mut self) -> InteractionId;
    /// Id of a queued input.
    fn queued(&mut self) -> String;
}

/// Time-ordered random ids; unique across threads and daemon restarts.
#[derive(Debug, Default)]
pub struct UuidMint;

impl IdMint for UuidMint {
    fn turn(&mut self) -> TurnId {
        TurnId(format!("turn_{}", uuid::Uuid::now_v7().simple()))
    }

    fn item(&mut self) -> ItemId {
        ItemId(format!("item_{}", uuid::Uuid::now_v7().simple()))
    }

    fn interaction(&mut self) -> InteractionId {
        InteractionId(format!("int_{}", uuid::Uuid::now_v7().simple()))
    }

    fn queued(&mut self) -> String {
        format!("q_{}", uuid::Uuid::now_v7().simple())
    }
}

/// Deterministic ids (`turn_1`, `item_1`, …) for tests and golden files.
#[derive(Debug, Default)]
pub struct SequentialMint {
    turns: u64,
    items: u64,
    interactions: u64,
    queued: u64,
}

impl SequentialMint {
    /// Continue numbering after `count` ids of each family, the way a restarted
    /// daemon must not reuse ids already in the journal.
    pub fn starting_after(count: u64) -> Self {
        Self { turns: count, items: count, interactions: count, queued: count }
    }
}

impl IdMint for SequentialMint {
    fn turn(&mut self) -> TurnId {
        self.turns += 1;
        TurnId(format!("turn_{}", self.turns))
    }

    fn item(&mut self) -> ItemId {
        self.items += 1;
        ItemId(format!("item_{}", self.items))
    }

    fn interaction(&mut self) -> InteractionId {
        self.interactions += 1;
        InteractionId(format!("int_{}", self.interactions))
    }

    fn queued(&mut self) -> String {
        self.queued += 1;
        format!("q_{}", self.queued)
    }
}
