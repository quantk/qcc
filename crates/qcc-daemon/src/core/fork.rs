//! Forking a thread from one of its turns: a new thread whose agent session is
//! made from the old one's, up to that turn, by the provider itself.

use qcc_domain::{ForkOrigin, ProviderInfo, Thread, ThreadId, TurnRecord, fork_blocker, turns};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{
    ThreadForkParams, ThreadResult, ThreadSpawnParams, ThreadTurnsParams, ThreadTurnsResult,
    TurnSummary,
};
use qcc_runtime::ForkStart;

use super::threads::SpawnOrigin;
use super::{Core, thread_not_found};

/// How long a prompt or a reply is in a list of turns.
const LINE_MAX_CHARS: usize = 120;

impl Core {
    /// The ended turns of a thread, as its journal tells them.
    fn turn_records(&self, id: &ThreadId) -> Result<(Thread, Vec<TurnRecord>), RpcError> {
        let thread = self.supervisor.thread(id).cloned().ok_or_else(|| thread_not_found(id))?;
        let events = self
            .store
            .events(id, 0)
            .map_err(|error| RpcError::new(codes::INTERNAL_ERROR, error.to_string()))?;
        Ok((thread, turns(&events)))
    }

    /// Why the thread cannot be forked from `record`; `None` when it can.
    fn blocker(&self, record: &TurnRecord) -> Option<String> {
        let info = self.host.provider(&record.provider).map(|(_, info)| info);
        let name = info.map_or_else(|| record.provider.to_string(), |info| info.name.clone());
        fork_blocker(record, &name, info.map(|info: &ProviderInfo| &info.capabilities))
    }

    pub(super) fn thread_turns(&self, p: ThreadTurnsParams) -> Result<ThreadTurnsResult, RpcError> {
        let (_, records) = self.turn_records(&p.thread_id)?;
        let turns = records
            .iter()
            .zip(1..)
            .map(|(record, number)| TurnSummary {
                turn_id: record.turn_id.clone(),
                number,
                status: record.status,
                prompt: record.prompt.as_deref().map(first_line),
                reply: record.reply.as_deref().map(first_line),
                fork_blocker: self.blocker(record),
            })
            .collect();
        Ok(ThreadTurnsResult { turns })
    }

    pub(super) fn thread_fork(&mut self, p: ThreadForkParams) -> Result<ThreadResult, RpcError> {
        let (source, records) = self.turn_records(&p.thread_id)?;
        let record = match &p.turn_id {
            Some(turn) => records
                .iter()
                .find(|record| &record.turn_id == turn)
                .ok_or_else(|| {
                    RpcError::new(
                        codes::INVALID_PARAMS,
                        format!("thread {} has no ended turn {turn}", source.id),
                    )
                })?
                .clone(),
            None => records
                .iter()
                .rev()
                .find(|record| self.blocker(record).is_none())
                .cloned()
                .ok_or_else(|| {
                    let why = records.last().and_then(|record| self.blocker(record));
                    RpcError::new(
                        codes::INVALID_PARAMS,
                        why.unwrap_or_else(|| {
                            format!("thread {} has no turn to fork from yet", source.id)
                        }),
                    )
                })?,
        };
        let why = self.blocker(&record);
        let (None, Some(point), Some(session)) =
            (why, record.point.clone(), record.session.clone())
        else {
            let why = self.blocker(&record).unwrap_or_default();
            return Err(RpcError::new(codes::INVALID_PARAMS, why));
        };
        let provider = record.provider.clone();
        let same_agent = provider == source.provider;
        // The fork may run with an agent the thread has left since; it runs as
        // strictly as the thread, as far as that agent can.
        let modes = self
            .host
            .provider(&provider)
            .map(|(_, info)| info.capabilities.permission_modes.clone())
            .unwrap_or_default();
        let permission_mode = source.permission_mode.and_then(|mode| mode.fit(&modes));
        let title = source.title.as_ref().map(|title| format!("{title} (fork)"));
        let fork = ForkStart {
            origin: ForkOrigin { thread: source.id.clone(), turn: record.turn_id, seq: record.seq },
            provider_session: session,
            point,
        };
        let params = ThreadSpawnParams {
            provider,
            cwd: source.cwd.clone(),
            environment: p.environment,
            base: None,
            environment_dir: None,
            branch: None,
            reuse_branch: false,
            prompt: p.prompt,
            attachments: Vec::new(),
            model: source.model.clone().filter(|_| same_agent),
            effort: source.effort.clone().filter(|_| same_agent),
            // Beside the thread, not under it: a fork is held to what the thread is
            // held to, and no more.
            parent: source.parent.clone(),
            project: source.project.clone(),
            permission_mode,
            skills_enabled: source.skills_enabled.clone(),
            skills_disabled: source.skills_disabled.clone(),
            started_by: None,
            hidden: false,
        };
        self.spawn_thread(params, SpawnOrigin::Fork(fork, title))
    }
}

fn first_line(text: &str) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    if line.chars().count() <= LINE_MAX_CHARS {
        return line.to_owned();
    }
    let mut short: String = line.chars().take(LINE_MAX_CHARS - 1).collect();
    short.push('…');
    short
}
