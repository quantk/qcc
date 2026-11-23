//! The timeline owner.
//!
//! * [`DeltaAssembler`] turns a bridge's deltas into canonical event payloads. It mints
//!   every turn, item and interaction id and owns every lifecycle invariant.
//! * [`Timeline`] is the fold of a thread's events: what is open right now. It doubles
//!   as the grammar checker for assembled timelines.
//! * [`ThreadEngine`] is one thread: assembler + projection + sequence numbers.
//! * [`Supervisor`] holds the engines of all threads.
//!
//! Everything here is synchronous and free of IO: callers pass in the clock and decide
//! where events are persisted and who is told about them. Nothing branches on a provider id.

mod assembler;
mod engine;
mod mint;
mod supervisor;
mod timeline;

pub use assembler::{DeltaAssembler, GrammarViolation};
pub use engine::{EngineError, ThreadEngine};
pub use mint::{IdMint, SequentialMint, UuidMint};
pub use supervisor::{ForkStart, NewThread, Supervisor, SupervisorError};
pub use timeline::{Timeline, TimelineViolation};
