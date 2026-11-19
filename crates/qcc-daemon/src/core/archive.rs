//! Archiving threads, alone or with every thread started under them.
//!
//! A tree is archived one thread at a time, deepest first, through the same steps
//! as a single thread. Before the first step the whole tree is held (it takes no
//! input and gets no new children), so the set of threads does not change under
//! the walk. A thread that refuses keeps its ancestors: an archived thread never
//! has live threads under it.

use std::collections::{HashMap, HashSet};

use qcc_domain::{EventPayload, ThreadId};
use qcc_plugin_host::ProviderRequest;
use qcc_protocol::jsonrpc::{RpcError, codes};
use qcc_protocol::methods::{ArchiveRefusal, Empty, ThreadArchiveParams, ThreadArchiveResult};
use serde_json::Value;
use tokio::sync::oneshot;

use super::{Action, Command, Core, Responder, now_ms, thread_not_found};

/// What one step of a tree archive came to.
pub(crate) struct StepOutcome {
    /// The thread was in a turn, which archiving stopped.
    pub(crate) working: bool,
    pub(crate) result: Result<Value, RpcError>,
}

/// What a tree archive did, gathered by the task that walks the tree.
#[derive(Default)]
pub(crate) struct TreeReport {
    archived: Vec<ThreadId>,
    stopped: Vec<ThreadId>,
    refused: Vec<ArchiveRefusal>,
    kept: Vec<ThreadId>,
}

fn archive_result(thread: qcc_domain::Thread) -> ThreadArchiveResult {
    ThreadArchiveResult {
        thread,
        archived: Vec::new(),
        stopped: Vec::new(),
        refused: Vec::new(),
        kept: Vec::new(),
    }
}

impl Core {
    pub(super) fn thread_archive(&mut self, p: ThreadArchiveParams, responder: Responder) {
        let ThreadArchiveParams { thread_id, force, descendants } = p;
        if descendants {
            self.archive_tree(thread_id, force, responder);
        } else {
            self.archive_one(&thread_id, force, false, responder);
        }
    }

    /// Release the session, then remove the environment, then mark the thread.
    ///
    /// `in_tree`: the thread is a step of a tree archive, which holds it already and
    /// stops it if it is working. Alone, a working thread is refused: the user
    /// stops what they are looking at themselves.
    fn archive_one(&mut self, id: &ThreadId, force: bool, in_tree: bool, responder: Responder) {
        let Some(thread) = self.supervisor.thread(id).cloned() else {
            return responder.reply::<Empty>(Err(thread_not_found(id)));
        };
        if thread.archived {
            return responder.reply(Ok(archive_result(thread)));
        }
        let held = self.in_transition(id);
        let Ok(engine) = self.supervisor.engine(id) else {
            return responder.reply::<Empty>(Err(thread_not_found(id)));
        };
        if held {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "the provider session is still starting or stopping; try again",
            )));
        }
        if !in_tree && (engine.busy() || self.archiving.contains(id)) {
            return responder.reply::<Empty>(Err(RpcError::new(
                codes::THREAD_BUSY,
                "the thread is working; stop it before archiving it",
            )));
        }
        let dropped = engine.drop_queued(now_ms());
        // Releasing the session ends a running turn too: that is how a working
        // thread of a tree is stopped.
        let live = engine.session_live();
        self.commit(dropped);
        self.queued.remove(id);
        self.archiving.insert(id.clone());
        if live {
            self.call_provider(id, Action::Archive { force }, Some(responder), |thread| {
                ProviderRequest::Stop { thread }
            });
        } else {
            self.remove_environment(id.clone(), force, responder);
        }
    }

    pub(super) fn finish_archive(&mut self, id: &ThreadId, removed: bool, responder: Responder) {
        self.archiving.remove(id);
        self.sessions.remove(id);
        let Ok(engine) = self.supervisor.engine(id) else {
            return responder.reply::<Empty>(Err(thread_not_found(id)));
        };
        let now = now_ms();
        let mut events = Vec::new();
        if removed {
            events.extend(engine.record(EventPayload::EnvironmentRemoved, now));
        }
        events.extend(engine.record(EventPayload::ThreadArchived, now));
        let thread = engine.thread().clone();
        self.commit(events);
        responder.reply(Ok(archive_result(thread)));
    }

    /// The threads under `root` that are not archived, deepest first, with their
    /// parents. An archived thread in between is looked through: what is under it
    /// still belongs to the tree.
    fn live_descendants(&self, root: &ThreadId) -> Vec<(ThreadId, ThreadId)> {
        let mut children: HashMap<&ThreadId, Vec<&ThreadId>> = HashMap::new();
        for thread in self.supervisor.threads() {
            if let Some(parent) = &thread.parent {
                children.entry(parent).or_default().push(&thread.id);
            }
        }
        // Breadth first gives every thread after its parent; reversed, before it.
        let mut order = Vec::new();
        let mut seen = HashSet::from([root]);
        let mut next = vec![root];
        while !next.is_empty() {
            let mut below = Vec::new();
            for parent in next {
                for child in children.get(parent).into_iter().flatten() {
                    // A cycle cannot be made through the API; this is only a backstop.
                    if seen.insert(child) {
                        order.push(((*child).clone(), parent.clone()));
                        below.push(*child);
                    }
                }
            }
            next = below;
        }
        order.reverse();
        order.retain(|(id, _)| self.supervisor.thread(id).is_some_and(|t| !t.archived));
        order
    }

    /// Archive `root` and every live thread under it.
    fn archive_tree(&mut self, root: ThreadId, force: bool, responder: Responder) {
        let Some(thread) = self.supervisor.thread(&root).cloned() else {
            return responder.reply::<Empty>(Err(thread_not_found(&root)));
        };
        let members = self.live_descendants(&root);
        // Everything that would refuse at once is checked before anything is
        // touched, so a busy tree is refused whole.
        let busy = |message: String| RpcError::new(codes::THREAD_BUSY, message);
        if !thread.archived {
            let working = self.supervisor.engine_ref(&root).is_some_and(|engine| engine.busy());
            if working || self.archiving.contains(&root) || self.in_transition(&root) {
                return responder.reply::<Empty>(Err(busy(
                    "the thread is working; stop it before archiving it".into(),
                )));
            }
        }
        for (id, _) in &members {
            if self.archiving.contains(id) {
                return responder
                    .reply::<Empty>(Err(busy(format!("thread {id} is being archived already"))));
            }
            if self.in_transition(id) {
                return responder.reply::<Empty>(Err(busy(format!(
                    "the session of thread {id} is still starting or stopping; try again"
                ))));
            }
        }
        let mut steps: Vec<(ThreadId, Option<ThreadId>)> =
            members.into_iter().map(|(id, parent)| (id, Some(parent))).collect();
        if !thread.archived {
            steps.push((root.clone(), None));
        }
        // Held from now on: no input, no queue, no new children, no other archive.
        for (id, _) in &steps {
            self.archiving.insert(id.clone());
        }
        let commands = self.commands.clone();
        tokio::spawn(async move {
            let mut report = TreeReport::default();
            // Threads with a refusal under them.
            let mut blocked: HashSet<ThreadId> = HashSet::new();
            let ids: Vec<ThreadId> = steps.iter().map(|(id, _)| id.clone()).collect();
            for (id, parent) in steps {
                if blocked.contains(&id) {
                    report.kept.push(id);
                    blocked.extend(parent);
                    continue;
                }
                let (reply, outcome) = oneshot::channel();
                let step = Command::ArchiveStep { thread: id.clone(), force, reply };
                if commands.send(step).is_err() {
                    return;
                }
                // The daemon is shutting down.
                let Ok(StepOutcome { working, result }) = outcome.await else { return };
                match result {
                    Ok(_) => {
                        if working {
                            report.stopped.push(id.clone());
                        }
                        report.archived.push(id);
                    }
                    Err(error) => {
                        let RpcError { code, message, .. } = error;
                        report.refused.push(ArchiveRefusal { thread_id: id, code, message });
                        blocked.extend(parent);
                    }
                }
            }
            let _ = commands.send(Command::TreeArchived { root, members: ids, report, responder });
        });
    }

    /// One thread of a tree archive.
    pub(super) fn archive_step(
        &mut self,
        id: ThreadId,
        force: bool,
        reply: oneshot::Sender<StepOutcome>,
    ) {
        let working = self.supervisor.engine_ref(&id).is_some_and(|engine| engine.busy());
        let (responder, answer) = Responder::internal();
        self.archive_one(&id, force, true, responder);
        tokio::spawn(async move {
            if let Ok(result) = answer.await {
                let _ = reply.send(StepOutcome { working, result });
            }
        });
    }

    /// The walk of a tree is over: let go of what stays, and answer.
    pub(super) fn tree_archived(
        &mut self,
        root: ThreadId,
        members: Vec<ThreadId>,
        report: TreeReport,
        responder: Responder,
    ) {
        for id in &members {
            self.archiving.remove(id);
        }
        let Some(thread) = self.supervisor.thread(&root).cloned() else {
            return responder.reply::<Empty>(Err(thread_not_found(&root)));
        };
        let TreeReport { archived, stopped, refused, kept } = report;
        let archived = archived
            .iter()
            .filter(|id| **id != root)
            .filter_map(|id| self.supervisor.thread(id).cloned())
            .collect();
        let stopped = stopped.into_iter().filter(|id| *id != root).collect();
        responder.reply(Ok(ThreadArchiveResult { thread, archived, stopped, refused, kept }));
    }
}
