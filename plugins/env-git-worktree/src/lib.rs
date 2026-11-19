//! Git worktree environment.
//!
//! Every thread gets a `git worktree` of the repository it was started in, on a
//! branch of its own, so threads work side by side without touching the main
//! checkout or each other. The directory and the branch are the plugin's to choose
//! unless the user names them; what is theirs is then handled with more care (see
//! [`jobs::Prepare`] and [`jobs::Remove`]).
//!
//! Git is a process, and no host import blocks, so each request is a [`jobs`] state
//! machine: it names the commands to run, this file runs them and feeds their exits
//! back, and the request is answered through `completions` when the job finishes.

pub mod jobs;

use std::collections::{HashMap, VecDeque};

use qcc_plugin_sdk::lines::Tail;
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
    stderr: Tail,
    truncated: bool,
    timer: Option<u64>,
    timed_out: bool,
    /// The repository lock it holds, if any.
    lock: Option<String>,
}

#[derive(Default)]
struct Worktrees {
    /// Jobs by their completion token.
    jobs: HashMap<u64, (String, Job)>,
    running: HashMap<u64, Running>,
    /// Timeout timers and the processes they stop.
    timers: HashMap<u64, u64>,
    last_token: u64,
    /// Repositories with a command running that changes their worktrees, and the
    /// commands of other jobs waiting for it, in order.
    locks: HashMap<String, VecDeque<(u64, Cmd)>>,
}

fn key(thread: &str) -> String {
    format!("thread/{thread}")
}

fn record(thread: &str) -> Result<Option<Record>, String> {
    Ok(kv::get(&key(thread))?.and_then(|bytes| Record::from_json(&bytes)))
}

/// The folders this plugin made to hold worktrees the user placed, as a JSON list:
/// `tasks/BGT-123` of `tasks/BGT-123/<repository>`. Such a folder is removed with
/// the last worktree in it; one that was there before is the user's and stays.
const MADE_PARENTS: &str = "made-parents";

fn made_parents() -> Result<Vec<String>, String> {
    Ok(kv::get(MADE_PARENTS)?
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default())
}

fn set_made_parents(parents: &[String]) -> Result<(), String> {
    if parents.is_empty() {
        return kv::remove(MADE_PARENTS);
    }
    let bytes = serde_json::to_vec(parents).map_err(|error| error.to_string())?;
    kv::put(MADE_PARENTS, &bytes)
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
                        if let Some(lock) = &cmd.lock {
                            if let Some(waiting) = self.locks.get_mut(lock) {
                                waiting.push_back((token, cmd));
                                continue;
                            }
                            self.locks.insert(lock.clone(), VecDeque::new());
                        }
                        let (tag, lock) = (cmd.tag, cmd.lock.clone());
                        if let Err(error) = self.spawn(token, cmd) {
                            if let Some(lock) = lock {
                                self.release(&lock);
                            }
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

    /// A command holding `lock` is over: start the next one waiting for it, if its
    /// job is still on.
    fn release(&mut self, lock: &str) {
        let Some(mut waiting) = self.locks.remove(lock) else { return };
        while let Some((job, cmd)) = waiting.pop_front() {
            if self.jobs.contains_key(&job) {
                // It takes the lock over, and the rest wait for it in turn.
                self.locks.insert(lock.to_owned(), VecDeque::new());
                let lock = lock.to_owned();
                let tag = cmd.tag;
                let started = self.spawn(job, cmd);
                if let Some(queue) = self.locks.get_mut(&lock) {
                    queue.extend(waiting);
                }
                if let Err(error) = started {
                    self.release(&lock);
                    let exit = Exit { stderr: error, ..Exit::default() };
                    if let Some((_, owner)) = self.jobs.get_mut(&job) {
                        let step = owner.on_exit(tag, exit);
                        self.advance(job, step);
                    }
                }
                return;
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
        let lock = cmd.lock;
        self.running.insert(
            process,
            Running {
                job,
                tag: cmd.tag,
                stdout: Vec::new(),
                limit: cmd.limit,
                stderr: Tail::new(STDERR_TAIL),
                truncated: false,
                timer,
                timed_out: false,
                lock,
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
            let Some(run) = self.running.remove(&process) else { continue };
            if let Some(timer) = run.timer {
                timers::cancel(timer);
                self.timers.remove(&timer);
            }
            // Its exit will not be heard, so the lock is given up now.
            if let Some(lock) = run.lock {
                self.release(&lock);
            }
        }
        let outcome = result.and_then(|answer| {
            Ok(match answer {
                Answer::Ready { record, made_parent } => {
                    kv::put(&key(&thread), &record.to_json())?;
                    if made_parent {
                        let mut parents = made_parents()?;
                        if !parents.iter().any(|parent| parent == record.parent()) {
                            parents.push(record.parent().to_owned());
                            set_made_parents(&parents)?;
                        }
                    }
                    Outcome::EnvironmentReady(record.cwd)
                }
                Answer::Removed { parent_gone } => {
                    let removed = record(&thread)?;
                    kv::remove(&key(&thread))?;
                    if let Some(removed) = removed.filter(|removed| parent_gone && removed.placed) {
                        let mut parents = made_parents()?;
                        parents.retain(|parent| parent != removed.parent());
                        set_made_parents(&parents)?;
                    }
                    Outcome::Done
                }
                Answer::Changes(changes) => Outcome::EnvironmentChanges(changes),
                Answer::Delivered(summary) => Outcome::EnvironmentDelivered(summary),
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
        EnvironmentInfo {
            id: ENVIRONMENT.into(),
            name: "Git worktree".into(),
            // Any git checkout can say what is uncommitted in it.
            inspects_directories: true,
            // The worktree can go where the user says, on a branch they name.
            custom_placement: true,
        }
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
        )
        .carrying(request.carry_changes)
        .placed(request.dir, request.branch, request.reuse_branch);
        self.begin(request.thread, Job::Prepare(job))
    }

    fn remove(&mut self, request: EnvironmentRemove) -> Result<Reply<()>, String> {
        // Nothing of ours to remove.
        let Some(record) = record(&request.thread)? else { return Ok(Reply::Now(())) };
        // A folder the user had before the worktree is theirs to keep, even empty.
        let tidy = record.placed && made_parents()?.iter().any(|parent| parent == record.parent());
        let job = jobs::Remove::new(record, request.force).tidying_parent(tidy);
        self.begin(request.thread, Job::Remove(job))
    }

    fn changes(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentChanges>, String> {
        if query.inspect {
            let job = jobs::Inspect::new(query.cwd, false, query.paths, query.scope);
            return self.begin(query.thread, Job::Inspect(job));
        }
        let record = known(&query.thread)?;
        self.begin(query.thread, Job::Changes(jobs::Changes::new(record, query.paths, query.scope)))
    }

    fn diff(&mut self, query: EnvironmentQuery) -> Result<Reply<EnvironmentDiff>, String> {
        if query.inspect {
            let job = jobs::Inspect::new(query.cwd, true, query.paths, query.scope);
            return self.begin(query.thread, Job::Inspect(job));
        }
        let record = known(&query.thread)?;
        self.begin(query.thread, Job::Diff(jobs::Diff::new(record, query.paths, query.scope)))
    }

    fn deliver(&mut self, request: EnvironmentDeliver) -> Result<Reply<String>, String> {
        let record = known(&request.thread)?;
        let merge = request.mode == DeliverMode::Merge;
        let job = jobs::Deliver::new(record, request.thread.clone(), merge);
        self.begin(request.thread, Job::Deliver(job))
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
                run.stderr.push(&data);
            }
            ProcessEvent::Exit { process, code } => {
                let Some(run) = self.running.remove(&process) else { return };
                if let Some(timer) = run.timer {
                    timers::cancel(timer);
                    self.timers.remove(&timer);
                }
                if let Some(lock) = &run.lock {
                    self.release(lock);
                }
                let exit = Exit {
                    code,
                    stdout: run.stdout,
                    stderr: run.stderr.as_str().to_owned(),
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
