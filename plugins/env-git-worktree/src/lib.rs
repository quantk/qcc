//! Git worktree environment.
//!
//! Every thread gets a `git worktree` of the repository it was started in, on a
//! branch of its own, so threads work side by side without touching the main
//! checkout or each other.
//!
//! Git is a process, and no host import blocks, so each request is a [`jobs`] state
//! machine: it names the commands to run, this file runs them and feeds their exits
//! back, and the request is answered through `completions` when the job finishes.

pub mod jobs;

use std::collections::{HashMap, VecDeque};

use qcc_plugin_sdk::prelude::*;

use jobs::{Answer, Cmd, Exit, Job, Record, Step};

const ENVIRONMENT: &str = "git-worktree";
/// How much of a command's standard error is kept: the end, where the error is.
const STDERR_TAIL: usize = 4096;

/// A command of a job that has not exited yet.
struct Running {
    job: u64,
    tag: u32,
    stdout: Vec<u8>,
    limit: usize,
    stderr: Vec<u8>,
    truncated: bool,
    timer: Option<u64>,
    timed_out: bool,
}

#[derive(Default)]
struct Worktrees {
    /// Jobs by their completion token.
    jobs: HashMap<u64, (String, Job)>,
    running: HashMap<u64, Running>,
    /// Timeout timers and the processes they stop.
    timers: HashMap<u64, u64>,
    last_token: u64,
}

fn key(thread: &str) -> String {
    format!("thread/{thread}")
}

fn record(thread: &str) -> Result<Option<Record>, String> {
    Ok(kv::get(&key(thread))?.and_then(|bytes| Record::from_json(&bytes)))
}

fn known(thread: &str) -> Result<Record, String> {
    record(thread)?.ok_or_else(|| format!("no worktree was made for thread {thread}"))
}

impl Worktrees {
    /// Start a job. The request it serves is answered when the job finishes.
    fn begin<T>(&mut self, thread: String, mut job: Job) -> Result<Reply<T>, String> {
        self.last_token += 1;
        let token = self.last_token;
        let step = job.start();
        self.jobs.insert(token, (thread, job));
        self.advance(token, step);
        Ok(Reply::Later(token))
    }

    /// Carry out a step, and the steps that follow from commands that could not
    /// even be started.
    fn advance(&mut self, token: u64, step: Step) {
        let mut steps = VecDeque::from([step]);
        while let Some(step) = steps.pop_front() {
            match step {
                Step::Wait => {}
                Step::Run(cmds) => {
                    for cmd in cmds {
                        let tag = cmd.tag;
                        if let Err(error) = self.spawn(token, cmd) {
                            let exit = Exit { stderr: error, ..Exit::default() };
                            if let Some((_, job)) = self.jobs.get_mut(&token) {
                                steps.push_back(job.on_exit(tag, exit));
                            }
                        }
                    }
                }
                Step::Finish(result) => {
                    self.finish(token, result);
                    return;
                }
            }
        }
    }

    fn spawn(&mut self, job: u64, cmd: Cmd) -> Result<(), String> {
        let process = process::spawn(&process::SpawnOptions {
            program: cmd.program.to_owned(),
            args: cmd.args,
            env: cmd.env,
            cwd: cmd.cwd,
        })?;
        // Nothing here reads standard input; a script that tries gets end of file.
        let _ = process::close_stdin(process);
        // A command without its timeout could hang the job forever.
        let timer = match timers::start(cmd.timeout_ms) {
            Ok(timer) => timer,
            Err(error) => {
                let _ = process::kill(process);
                return Err(format!("cannot time {}: {error}", cmd.program));
            }
        };
        self.timers.insert(timer, process);
        let timer = Some(timer);
        self.running.insert(
            process,
            Running {
                job,
                tag: cmd.tag,
                stdout: Vec::new(),
                limit: cmd.limit,
                stderr: Vec::new(),
                truncated: false,
                timer,
                timed_out: false,
            },
        );
        Ok(())
    }

    fn finish(&mut self, token: u64, result: Result<Answer, String>) {
        let Some((thread, _)) = self.jobs.remove(&token) else { return };
        // A job that ends early leaves nothing running behind it.
        let orphans: Vec<u64> =
            self.running.iter().filter(|(_, run)| run.job == token).map(|(id, _)| *id).collect();
        for process in orphans {
            let _ = process::kill(process);
            if let Some(timer) = self.running.remove(&process).and_then(|run| run.timer) {
                timers::cancel(timer);
                self.timers.remove(&timer);
            }
        }
        let outcome = result.and_then(|answer| {
            Ok(match answer {
                Answer::Ready(record) => {
                    kv::put(&key(&thread), &record.to_json())?;
                    Outcome::EnvironmentReady(record.cwd)
                }
                Answer::Removed => {
                    kv::remove(&key(&thread))?;
                    Outcome::Done
                }
                Answer::Changes(changes) => Outcome::EnvironmentChanges(changes),
                Answer::Diff { diff, truncated } => {
                    Outcome::EnvironmentDiff(EnvironmentDiff { diff, truncated })
                }
            })
        });
        if let Err(error) = &outcome {
            log::warn(format!("thread {thread}: {error}"));
        }
        let answer = outcome.as_ref().map_err(String::as_str);
        if let Err(error) = completions::complete(token, answer) {
            log::error(format!("cannot answer request {token}: {error}"));
        }
    }
}

impl Environment for Worktrees {
    fn info(&self) -> EnvironmentInfo {
        EnvironmentInfo { id: ENVIRONMENT.into(), name: "Git worktree".into() }
    }

    fn prepare(&mut self, request: EnvironmentPrepare) -> Result<Reply<String>, String> {
        // Asked again for a thread that has its worktree: it is the same one.
        if let Some(record) = record(&request.thread)? {
            return Ok(Reply::Now(record.cwd));
        }
        let job = jobs::Prepare::new(
            request.thread.clone(),
            request.source_dir,
            request.root,
            request.base,
        );
        self.begin(request.thread, Job::Prepare(job))
    }

    fn remove(&mut self, request: EnvironmentRemove) -> Result<Reply<()>, String> {
        // Nothing of ours to remove.
        let Some(record) = record(&request.thread)? else { return Ok(Reply::Now(())) };
        self.begin(request.thread, Job::Remove(jobs::Remove::new(record, request.force)))
    }

    fn changes(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentChanges>, String> {
        let record = known(&query.thread)?;
        self.begin(query.thread, Job::Changes(jobs::Changes::new(record)))
    }

    fn diff(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentDiff>, String> {
        let record = known(&query.thread)?;
        self.begin(query.thread, Job::Diff(jobs::Diff::new(record)))
    }
}

impl Plugin for Worktrees {
    fn environments(&mut self) -> Vec<&mut dyn Environment> {
        vec![self]
    }

    fn on_process(&mut self, event: ProcessEvent) {
        match event {
            ProcessEvent::Stdout { process, data } => {
                let Some(run) = self.running.get_mut(&process) else { return };
                let room = run.limit.saturating_sub(run.stdout.len());
                run.stdout.extend_from_slice(&data[..data.len().min(room)]);
                if data.len() > room && !run.truncated {
                    // Enough: the rest would only be thrown away.
                    run.truncated = true;
                    let _ = process::kill(process);
                }
            }
            ProcessEvent::Stderr { process, data } => {
                let Some(run) = self.running.get_mut(&process) else { return };
                run.stderr.extend_from_slice(&data);
                let excess = run.stderr.len().saturating_sub(STDERR_TAIL);
                run.stderr.drain(..excess);
            }
            ProcessEvent::Exit { process, code } => {
                let Some(run) = self.running.remove(&process) else { return };
                if let Some(timer) = run.timer {
                    timers::cancel(timer);
                    self.timers.remove(&timer);
                }
                let exit = Exit {
                    code,
                    stdout: run.stdout,
                    stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
                    truncated: run.truncated,
                    timed_out: run.timed_out,
                };
                let Some((_, job)) = self.jobs.get_mut(&run.job) else { return };
                let step = job.on_exit(run.tag, exit);
                self.advance(run.job, step);
            }
        }
    }

    fn on_timer(&mut self, timer: u64) {
        let Some(process) = self.timers.remove(&timer) else { return };
        if let Some(run) = self.running.get_mut(&process) {
            run.timed_out = true;
            run.timer = None;
            let _ = process::kill(process);
        }
    }
}

qcc_plugin_sdk::export_plugin!(Worktrees);
