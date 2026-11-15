use std::collections::HashMap;

use qcc_domain::{
    EventPayload, PermissionMode, ProviderId, Thread, ThreadEnvironment, ThreadEvent, ThreadId,
    ThreadStatus,
};

use crate::engine::ThreadEngine;
use crate::mint::IdMint;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SupervisorError {
    #[error("thread {0} does not exist")]
    UnknownThread(ThreadId),
    #[error("thread {0} already exists")]
    ThreadExists(ThreadId),
}

pub struct NewThread {
    pub id: ThreadId,
    pub provider: ProviderId,
    pub cwd: String,
    pub environment: Option<ThreadEnvironment>,
    pub model: Option<String>,
    pub parent: Option<ThreadId>,
    pub permission_mode: Option<PermissionMode>,
}

/// All threads of a daemon. Owns one [`ThreadEngine`] per thread.
pub struct Supervisor {
    engines: HashMap<ThreadId, ThreadEngine>,
    mint: Box<dyn Fn() -> Box<dyn IdMint> + Send>,
}

impl Supervisor {
    /// `mint` makes the id source of each thread's assembler.
    pub fn new(mint: impl Fn() -> Box<dyn IdMint> + Send + 'static) -> Self {
        Self { engines: HashMap::new(), mint: Box::new(mint) }
    }

    pub fn create(
        &mut self,
        new: NewThread,
        now: u64,
    ) -> Result<Vec<ThreadEvent>, SupervisorError> {
        if self.engines.contains_key(&new.id) {
            return Err(SupervisorError::ThreadExists(new.id));
        }
        let thread = Thread {
            id: new.id.clone(),
            provider: new.provider,
            cwd: new.cwd,
            environment: new.environment,
            model: new.model,
            parent: new.parent,
            permission_mode: new.permission_mode,
            title: None,
            status: ThreadStatus::Idle,
            provider_session: None,
            created_at: now,
            updated_at: now,
            last_seq: 0,
            queued_inputs: 0,
            pending_interactions: Vec::new(),
            activity: None,
            turn_started_at: None,
            last_turn_status: None,
            archived: false,
            handover: false,
        };
        let (engine, events) = ThreadEngine::create(thread, (self.mint)(), now);
        self.engines.insert(new.id, engine);
        Ok(events)
    }

    /// Adopt a persisted thread after a restart; see [`ThreadEngine::recover`].
    pub fn recover<'a>(
        &mut self,
        thread: Thread,
        history: impl IntoIterator<Item = &'a EventPayload>,
        now: u64,
    ) -> Vec<ThreadEvent> {
        let id = thread.id.clone();
        let (engine, events) = ThreadEngine::recover(thread, history, (self.mint)(), now);
        self.engines.insert(id, engine);
        events
    }

    pub fn thread(&self, id: &ThreadId) -> Option<&Thread> {
        self.engines.get(id).map(ThreadEngine::thread)
    }

    pub fn threads(&self) -> impl Iterator<Item = &Thread> {
        self.engines.values().map(ThreadEngine::thread)
    }

    pub fn engine(&mut self, id: &ThreadId) -> Result<&mut ThreadEngine, SupervisorError> {
        self.engines.get_mut(id).ok_or_else(|| SupervisorError::UnknownThread(id.clone()))
    }
}
