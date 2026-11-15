//! The plumbing of a provider whose agent is a process speaking lines on stdio.
//!
//! The dialect stays in the plugin: a [`LineBridge`] turns lines into an [`Output`]
//! and does no I/O. [`Sessions`] does the rest the same way for every such provider:
//! it maps processes to threads, reassembles lines, writes to the agent, emits deltas
//! and reports the process going away. [`Probes`] runs an agent for a single
//! question, such as which models it offers.

use std::collections::HashMap;

use crate::lines::{LineBuffer, Tail};
use crate::types::{Delta, Outcome};
use crate::{ProcessEvent, completions, deltas, log, process, timers};

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
    /// Call [`LineBridge::on_wake`] after this many milliseconds: for a bridge that
    /// has to ask the agent again later. Needs the `timers` permission.
    pub wake_after_ms: Option<u64>,
}

/// The dialect of one agent, as [`Sessions`] drives it.
pub trait LineBridge {
    /// One complete line from the agent's stdout.
    fn on_line(&mut self, line: &str) -> Output;

    /// The process went away by itself, after all its output. `stderr` is the last
    /// line it wrote there, which usually says why.
    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Vec<Delta>;

    /// The time asked for with [`Output::wake_after_ms`] has passed.
    fn on_wake(&mut self) -> Output {
        Output::default()
    }
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
    /// Timers of bridges waiting to be woken, and the thread of each.
    wakes: HashMap<u64, String>,
}

impl<B> Default for Sessions<B> {
    fn default() -> Self {
        Self { sessions: HashMap::new(), wakes: HashMap::new() }
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
        } else if let Some(delay) = output.wake_after_ms {
            match timers::start(delay) {
                Ok(timer) => {
                    self.wakes.insert(timer, thread.to_owned());
                }
                Err(error) => result = result.and(Err(error)),
            }
        }
        result
    }

    /// Wake the bridge a timer was started for. Returns whether the timer was one of
    /// these sessions'; a session that ended meanwhile is not woken.
    pub fn on_timer(&mut self, timer: u64) -> bool {
        let Some(thread) = self.wakes.remove(&timer) else { return false };
        let Ok(bridge) = self.bridge(&thread) else { return true };
        let output = bridge.on_wake();
        if let Err(error) = self.apply(&thread, output) {
            log::error(format!("thread {thread}: {error}"));
        }
        true
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

/// How long a probe may run before it is killed and its request answered with an
/// error. Asking an agent what it offers takes a second or two; one that hangs
/// (waiting on a login, say) must not keep its caller waiting.
pub const PROBE_TIMEOUT_MS: u64 = 20_000;

/// Reads the answer to one question out of a short-lived agent process.
pub trait ProbeParser {
    /// One complete line from stdout. `Some` ends the probe with that answer.
    fn on_line(&mut self, line: &str) -> Option<Result<Outcome, String>>;

    /// The process ended before [`ProbeParser::on_line`] answered.
    fn exited(&mut self, code: Option<i32>, stderr: Option<&str>) -> Result<Outcome, String>;
}

struct Probe {
    stdout: LineBuffer,
    stderr: Tail,
    timer: Option<u64>,
    parser: Box<dyn ProbeParser>,
}

/// Processes run to answer a deferred request rather than to hold a session: the
/// agent is asked one thing, its answer goes to [`completions::complete`] and the
/// process is killed. The request's token is the process id.
#[derive(Default)]
pub struct Probes {
    running: HashMap<u64, Probe>,
}

impl Probes {
    /// Run a probe and return the token to answer the request `deferred` with.
    /// `stdin` is written right away, one line each; with nothing to write, stdin is
    /// closed, for programs that would otherwise wait on it.
    ///
    /// The timeout needs the `timers` permission. Without it the probe runs until
    /// it answers or exits.
    ///
    /// # Errors
    ///
    /// When the program cannot be started or written to.
    pub fn start(
        &mut self,
        options: &process::SpawnOptions,
        stdin: &[String],
        parser: Box<dyn ProbeParser>,
    ) -> Result<u64, String> {
        let process = process::spawn(options)?;
        let written = if stdin.is_empty() {
            process::close_stdin(process)
        } else {
            stdin
                .iter()
                .try_for_each(|line| process::write_stdin(process, format!("{line}\n").as_bytes()))
        };
        if let Err(error) = written {
            let _ = process::kill(process);
            return Err(error);
        }
        let timer = timers::start(PROBE_TIMEOUT_MS).ok();
        let probe = Probe {
            stdout: LineBuffer::default(),
            stderr: Tail::new(STDERR_TAIL_BYTES),
            timer,
            parser,
        };
        self.running.insert(process, probe);
        Ok(process)
    }

    /// Hand over a process event. Returns it back when it is not a probe's.
    pub fn on_process(&mut self, event: ProcessEvent) -> Option<ProcessEvent> {
        let process = match &event {
            ProcessEvent::Stdout { process, .. }
            | ProcessEvent::Stderr { process, .. }
            | ProcessEvent::Exit { process, .. } => *process,
        };
        let Some(probe) = self.running.get_mut(&process) else { return Some(event) };
        match event {
            ProcessEvent::Stdout { data, .. } => {
                for line in probe.stdout.push(&data) {
                    if let Some(answer) = probe.parser.on_line(&line) {
                        self.finish(process, answer);
                        break;
                    }
                }
            }
            ProcessEvent::Stderr { data, .. } => probe.stderr.push(&data),
            ProcessEvent::Exit { code, .. } => {
                let answer =
                    match probe.stdout.finish().and_then(|line| probe.parser.on_line(&line)) {
                        Some(answer) => answer,
                        None => {
                            let stderr = probe.stderr.last_line().map(str::to_owned);
                            probe.parser.exited(code, stderr.as_deref())
                        }
                    };
                self.finish(process, answer);
            }
        }
        None
    }

    /// Hand over a fired timer. Returns whether it was a probe's.
    pub fn on_timer(&mut self, timer: u64) -> bool {
        let Some(process) =
            self.running.iter().find(|(_, probe)| probe.timer == Some(timer)).map(|(id, _)| *id)
        else {
            return false;
        };
        let seconds = PROBE_TIMEOUT_MS / 1000;
        self.finish(process, Err(format!("the agent did not answer within {seconds} s")));
        true
    }

    fn finish(&mut self, process: u64, answer: Result<Outcome, String>) {
        let Some(probe) = self.running.remove(&process) else { return };
        if let Some(timer) = probe.timer {
            timers::cancel(timer);
        }
        // Killing a process that already exited is harmless.
        let _ = process::kill(process);
        if let Err(error) = completions::complete(process, answer.as_ref().map_err(String::as_str))
        {
            log::warn(format!("could not answer probe {process}: {error}"));
        }
    }
}
