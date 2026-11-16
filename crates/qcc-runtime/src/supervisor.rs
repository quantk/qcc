use std::collections::HashMap;

use qcc_domain::{
    EventPayload, PermissionMode, ProjectId, ProviderId, Thread, ThreadEnvironment, ThreadEvent,
    ThreadId, ThreadStatus, ThreadUsage,
};

use crate::engine::{ThreadEngine, sorted};
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
    pub effort: Option<String>,
    pub parent: Option<ThreadId>,
    pub project: Option<ProjectId>,
    pub permission_mode: Option<PermissionMode>,
    /// Skills offered on request it gets, and skills of every session it goes without.
    pub skills_enabled: Vec<String>,
    pub skills_disabled: Vec<String>,
    pub started_by: Option<ThreadId>,
    pub hidden: bool,
}

impl NewThread {
    /// The thread as it is the moment it is created.
    pub fn into_thread(self, now: u64) -> Thread {
        Thread {
            id: self.id,
            provider: self.provider,
            cwd: self.cwd,
            environment: self.environment,
            model: self.model,
            effort: self.effort,
            parent: self.parent,
            started_by: self.started_by,
            hidden: self.hidden,
            project: self.project,
            permission_mode: self.permission_mode,
            title: None,
            status: ThreadStatus::Idle,
            provider_session: None,
            created_at: now,
            updated_at: now,
            last_seq: 0,
            queued_inputs: 0,
            pending_interactions: Vec::new(),
            mcp_servers: Vec::new(),
            mcp_disabled: Vec::new(),
            skills_enabled: sorted(self.skills_enabled),
            skills_disabled: sorted(self.skills_disabled),
            conversations: 0,
            activity: None,
            turn_started_at: None,
            last_turn_status: None,
            last_turn_at: None,
            usage: ThreadUsage::default(),
            archived: false,
            handover: false,
        }
    }
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
        let id = new.id.clone();
        let (engine, events) = ThreadEngine::create(new.into_thread(now), (self.mint)(), now);
        self.engines.insert(id, engine);
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

    /// The engine of a thread, to read from.
    pub fn engine_ref(&self, id: &ThreadId) -> Option<&ThreadEngine> {
        self.engines.get(id)
    }

    pub fn engine(&mut self, id: &ThreadId) -> Result<&mut ThreadEngine, SupervisorError> {
        self.engines.get_mut(id).ok_or_else(|| SupervisorError::UnknownThread(id.clone()))
    }
}
