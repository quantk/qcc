//! `thread.wait`: answering clients that wait for threads to settle.

use std::time::Duration;

use qcc_domain::{ThreadId, ThreadState, ThreadStatus};
use qcc_plugin_host::{PluginRequest, SettledThread, ThreadSummary};
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{Empty, ThreadWaitParams, ThreadWaitResult, WaitedThread};

use super::{Command, Core, Responder, Waiter, thread_not_found};

impl Core {
    /// Where a thread is, for someone waiting on it.
    pub(super) fn thread_state(&self, id: &ThreadId) -> Option<ThreadState> {
        let held = self.in_transition(id);
        let archiving = self.archiving.contains(id);
        let engine = self.supervisor.engine_ref(id)?;
        let thread = engine.thread();
        Some(if thread.archived {
            ThreadState::Archived
        } else if !thread.pending_interactions.is_empty() {
            ThreadState::Waiting
        } else if held || archiving || engine.busy() || engine.has_queued_input() {
            ThreadState::Working
        } else if thread.status == ThreadStatus::Failed {
            ThreadState::Failed
        } else {
            ThreadState::Idle
        })
    }

    pub(super) fn summary(&self, id: &ThreadId) -> Option<ThreadSummary> {
        let state = self.thread_state(id)?;
        let thread = self.supervisor.thread(id)?.clone();
        Some(ThreadSummary { thread, state })
    }

    fn last_message(&self, id: &ThreadId) -> Option<String> {
        match self.store.transcript(id, true) {
            Ok(mut entries) => entries.pop().map(|entry| entry.text),
            Err(error) => {
                tracing::error!(thread = %id, %error, "cannot read the transcript");
                None
            }
        }
    }

    pub(super) fn thread_wait(
        &mut self,
        connection: u64,
        p: ThreadWaitParams,
        responder: Responder,
    ) {
        let ThreadWaitParams { thread_ids, any, timeout_ms } = p;
        if thread_ids.is_empty() {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::INVALID_PARAMS,
                "name a thread to wait on",
            )));
        }
        if let Some(missing) = thread_ids.iter().find(|id| self.supervisor.thread(id).is_none()) {
            return responder.reply::<Empty>(Err(thread_not_found(missing)));
        }
        self.next_waiter += 1;
        let id = self.next_waiter;
        self.waiters.push(Waiter { id, connection, threads: thread_ids, any, responder });
        if let Some(timeout) = timeout_ms {
            let commands = self.commands.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(timeout)).await;
                let _ = commands.send(Command::WaitTimedOut { waiter: id });
            });
        }
        // Answered right away, after this command, if nothing needs to happen first.
    }

    pub(super) fn answer_wait(&mut self, waiter: Waiter, timed_out: bool) {
        let mut threads = Vec::new();
        for id in &waiter.threads {
            let (Some(state), Some(thread)) =
                (self.thread_state(id), self.supervisor.thread(id).cloned())
            else {
                continue;
            };
            let last_message = self.last_message(id);
            threads.push(WaitedThread { thread, state, last_message });
        }
        waiter.responder.reply(Ok(ThreadWaitResult { threads, timed_out }));
    }

    /// Answer the waits and fire the watches whose threads settled.
    pub(super) fn settle_waits(&mut self) {
        if self.waiters.is_empty() && self.watches.is_empty() {
            return;
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let (threads, any) = (self.waiters[index].threads.clone(), self.waiters[index].any);
            let mut settled =
                threads.iter().map(|id| self.thread_state(id).is_none_or(ThreadState::is_settled));
            let done = if any { settled.any(|s| s) } else { settled.all(|s| s) };
            if done {
                let waiter = self.waiters.remove(index);
                self.answer_wait(waiter, false);
            } else {
                index += 1;
            }
        }

        let mut index = 0;
        while index < self.watches.len() {
            let thread = self.watches[index].thread.clone();
            let Some(summary) = self.summary(&thread) else {
                self.watches.remove(index);
                continue;
            };
            if !summary.state.is_settled() {
                index += 1;
                continue;
            }
            let watch = self.watches.remove(index);
            let last_message = self.last_message(&thread);
            if let Some(plugin) = self.host.plugin(&watch.plugin) {
                let settled = SettledThread { summary, last_message };
                if plugin.post(PluginRequest::ThreadSettled(Box::new(settled))).is_err() {
                    tracing::warn!(plugin = watch.plugin, "a watching plugin is gone");
                }
            }
        }
    }
}
