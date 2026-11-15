//! The plumbing of a provider whose agent is a process speaking lines on stdio.
//!
//! The dialect stays in the plugin: a [`LineBridge`] turns lines into an [`Output`]
//! and does no I/O. [`Sessions`] does the rest the same way for every such provider:
//! it maps processes to threads, reassembles lines, writes to the agent, emits deltas
//! and reports the process going away.

use std::collections::HashMap;

use crate::lines::{LineBuffer, Tail};
use crate::types::Delta;
use crate::{ProcessEvent, deltas, log, process};

/// How much of an agent's stderr is kept to explain why it died.
const STDERR_TAIL_BYTES: usize = 4096;

/// What a bridge wants done after one input.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Output {
    /// Lines for the agent's stdin, without the newline.
    pub writes: Vec<String>,
    pub deltas: Vec<Delta>,
    /// The session cannot go on; the process should be killed.
    pub fatal: bool,
}

/// The dialect of one agent, as [`Sessions`] drives it.
pub trait LineBridge {
    /// One complete line from the agent's stdout.
    fn on_line(&mut self, line: &str) -> Output;

    /// The process went away by itself, after all its output. `stderr` is the last
    /// line it wrote there, which usually says why.
    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta>;
}

/// How an agent process ended, in words for the timeline.
pub fn exit_reason(agent: &str, code: Option<i32>, stderr: Option<&str>) -> String {
    let reason = match code {
        Some(code) => format!("{agent} exited with code {code}"),
        None => format!("{agent} was killed"),
    };
    match stderr {
        Some(stderr) => format!("{reason}: {stderr}"),
        None => reason,
    }
}

struct Session<B> {
    process: u64,
    stdout: LineBuffer,
    stderr: Tail,
    bridge: B,
}

/// The running sessions of a provider, one agent process per thread.
pub struct Sessions<B> {
    sessions: HashMap<String, Session<B>>,
}

impl<B> Default for Sessions<B> {
    fn default() -> Self {
        Self { sessions: HashMap::new() }
    }
}

impl<B: LineBridge> Sessions<B> {
    /// Attach `process` to `thread`, killing the process of a session it replaces.
    ///
    /// Insert as soon as the process is spawned: until then the process has no owner,
    /// and nothing kills it if the start fails halfway.
    pub fn insert(&mut self, thread: &str, process: u64, bridge: B) {
        let session = Session {
            process,
            stdout: LineBuffer::default(),
            stderr: Tail::new(STDERR_TAIL_BYTES),
            bridge,
        };
        if let Some(old) = self.sessions.insert(thread.to_owned(), session) {
            let _ = process::kill(old.process);
        }
    }

    /// Whether `process` is the agent of one of these sessions.
    pub fn owns(&self, process: u64) -> bool {
        self.sessions.values().any(|session| session.process == process)
    }

    /// The bridge of a running session.
    ///
    /// # Errors
    ///
    /// When no session runs for `thread`.
    pub fn bridge(&mut self, thread: &str) -> Result<&mut B, String> {
        self.sessions
            .get_mut(thread)
            .map(|session| &mut session.bridge)
            .ok_or_else(|| "the session is not running".to_owned())
    }

    /// End a session on our side: its process is killed and its bridge handed back,
    /// so the caller can say how it ended. Its process's last events are ignored.
    pub fn kill(&mut self, thread: &str) -> Option<B> {
        let session = self.sessions.remove(thread)?;
        let _ = process::kill(session.process);
        Some(session.bridge)
    }

    /// Carry out what the bridge decided: write to the agent, report to the core, and
    /// drop the session if it cannot go on.
    ///
    /// The deltas are emitted even when a write fails: the bridge has already moved
    /// on as if the writes had gone out, and the core must hear what it decided.
    ///
    /// # Errors
    ///
    /// The first write or emit that failed, or no session running for `thread`.
    pub fn apply(&mut self, thread: &str, output: Output) -> Result<(), String> {
        let session = self.sessions.get_mut(thread).ok_or("the session is not running")?;
        let mut result = Ok(());
        for line in output.writes {
            // Once a write failed the pipe is gone; the rest would fail the same way.
            if result.is_ok() {
                result = process::write_stdin(session.process, format!("{line}\n").as_bytes());
            }
        }
        if !output.deltas.is_empty() {
            result = result.and(deltas::emit(thread, &output.deltas));
        }
        if output.fatal {
            self.kill(thread);
        }
        result
    }

    /// Hand a process event to the session it belongs to. Errors are logged: nobody
    /// waits on a process event.
    pub fn on_process(&mut self, event: ProcessEvent) {
        let process = match &event {
            ProcessEvent::Stdout { process, .. }
            | ProcessEvent::Stderr { process, .. }
            | ProcessEvent::Exit { process, .. } => *process,
        };
        // A process whose session was stopped has nobody to report to.
        let Some(thread) = self
            .sessions
            .iter()
            .find(|(_, session)| session.process == process)
            .map(|(thread, _)| thread.clone())
        else {
            return;
        };
        if let Err(error) = self.process_event(&thread, event) {
            log::error(format!("thread {thread}: {error}"));
        }
    }

    fn process_event(&mut self, thread: &str, event: ProcessEvent) -> Result<(), String> {
        match event {
            ProcessEvent::Stdout { data, .. } => {
                let Some(session) = self.sessions.get_mut(thread) else { return Ok(()) };
                let mut result = Ok(());
                for line in session.stdout.push(&data) {
                    // A fatal line removes the session; the rest of the chunk is moot.
                    let Ok(bridge) = self.bridge(thread) else { break };
                    let output = bridge.on_line(&line);
                    // One failed line does not keep the next ones from the bridge.
                    result = result.and(self.apply(thread, output));
                }
                result
            }
            ProcessEvent::Stderr { data, .. } => {
                if let Some(session) = self.sessions.get_mut(thread) {
                    session.stderr.push(&data);
                }
                Ok(())
            }
            ProcessEvent::Exit { code, .. } => {
                let Some(mut session) = self.sessions.remove(thread) else { return Ok(()) };
                // The last line may have no newline after it; it is still the agent's
                // last word, often the one that ends the turn.
                let mut ending = Output::default();
                if let Some(line) = session.stdout.finish() {
                    ending = session.bridge.on_line(&line);
                }
                if !ending.fatal {
                    let stderr = session.stderr.last_line();
                    ending.deltas.extend(session.bridge.exited(code, stderr));
                }
                if ending.deltas.is_empty() {
                    return Ok(());
                }
                deltas::emit(thread, &ending.deltas)
            }
        }
    }
}
