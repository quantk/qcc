//! Reviewers: other agents look at what a thread changed, on their own.
//!
//! The user turns reviews on for a thread and picks reviewer profiles (markdown
//! files: who reviews and what to look for). When the trigger fires — by default
//! once the agent is done: a turn completed and no new one started for a while —
//! the plugin reads the thread's diff, and unless it was reviewed already, starts
//! one child thread per profile with the diff in its prompt. When all of them
//! answered, their findings become comments on the lines they are about and, if
//! the user chose so, one message to the agent.
//!
//! Nothing here is part of the core. The plugin hears of turns through the
//! `turn-events` permission, reads diffs and drives threads through `threads`,
//! leaves comments through `comments` and reads profiles through `files`. Turned
//! off, it leaves nothing behind but the comments and reviewer threads it made.

pub mod config;
pub mod diff;
pub mod findings;
pub mod ledger;
mod panel;
pub mod profile;
pub mod prompt;

use std::collections::HashMap;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::threads::SpawnOptions;

use crate::config::{Critic, Defaults, Progress, Round, RoundRecord, ThreadConfig, Trigger};
use crate::findings::{Finding, Report};
use crate::ledger::Status;
use crate::profile::{Delivery, Profile, Source};

const SKILL_NAME: &str = "qcc-reviewers";
/// This plugin's id, which the daemon records as the author of its comments.
const PLUGIN_ID: &str = "reviewers";
const SKILL_BODY: &str = include_str!("../SKILL.md");

/// `config/<thread>`: the reviews the user set up for a thread.
const CONFIG: &str = "config/";
/// `state/<thread>`: rounds, fingerprints and the round in flight.
const STATE: &str = "state/";
/// `critic/<reviewer thread>`: the thread it reviews, while it works.
const CRITIC: &str = "critic/";
/// `reviewer-of/<reviewer thread>`: the thread it reviewed, kept for its panel.
pub(crate) const REVIEWER_OF: &str = "reviewer-of/";

/// Where profiles live besides the built-in ones.
const GLOBAL_DIR: &str = "profiles";
const PROJECT_DIR: &str = ".qcc/reviewers";
/// The longest reply of a reviewer that did not follow the format, kept as one finding.
const MAX_LOOSE_REPLY: usize = 4_000;

/// Why a diff was asked for.
/// A round waiting for the list of changed files.
pub(crate) struct Pending {
    pub(crate) thread: String,
    /// The command that asked for the round, to be answered.
    token: Option<u64>,
    diff: String,
}

/// Why a reviewer may ask for approval: its agent cannot run read-only.
pub(crate) fn asks_note(profile: &str, provider: &str) -> String {
    format!(
        "{provider} cannot run read-only, so reviewer {profile} runs in ask mode and may ask \
         you for approval"
    )
}

/// The agents that can run a reviewer read-only.
pub(crate) fn read_only_providers() -> Vec<String> {
    threads::providers()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| p.capabilities.permission_modes.contains(&PermissionMode::ReadOnly))
        .map(|p| p.id)
        .collect()
}

pub(crate) enum DiffFor {
    /// To start a round; `token` answers the command that asked for it.
    Start { thread: String, token: Option<u64> },
    /// To tell whether a finished round's findings are stale.
    Finish { thread: String },
}

#[derive(Default)]
pub struct Reviewers {
    defaults: Defaults,
    /// Reviewers of the last daemon run are watched again once.
    rewatched: bool,
    /// The thread each pending timer is for.
    timers: HashMap<u64, String>,
    /// The pending timer of each thread.
    waiting: HashMap<String, u64>,
    diffs: HashMap<u64, DiffFor>,
    /// Rounds waiting for `threads.changes`, by request.
    changes: HashMap<u64, Pending>,
    next_token: u64,
    /// The findings picked in each thread's panel.
    picked: HashMap<String, std::collections::BTreeSet<String>>,
    /// Agents' models, for the panel.
    models: HashMap<String, panel::Models>,
    /// The agent each `threads.models` request is about.
    model_requests: HashMap<u64, String>,
}

pub(crate) fn load<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
    let bytes = kv::get(key).ok()??;
    serde_json::from_slice(&bytes).ok()
}

fn save<T: serde::Serialize>(key: &str, value: &T) {
    let bytes = serde_json::to_vec(value).expect("plugin state serializes");
    if let Err(error) = kv::put(key, &bytes) {
        log::error(format!("cannot keep {key}: {error}"));
    }
}

pub(crate) fn state_of(thread: &str) -> Progress {
    load(&format!("{STATE}{thread}")).unwrap_or_default()
}

pub(crate) fn save_state(thread: &str, state: &Progress) {
    save(&format!("{STATE}{thread}"), state);
}

/// The profiles a thread can use: built in, global, then its project's.
pub fn profiles_for(thread: &str) -> (Vec<Profile>, Vec<String>) {
    let mut found = Vec::new();
    let mut problems = Vec::new();
    for (file, text) in profile::BUILT_IN {
        match profile::parse(file, text, Source::BuiltIn) {
            Ok(profile) => found.push(profile),
            Err(error) => problems.push(error),
        }
    }
    let roots = [
        (files::Root::Own, GLOBAL_DIR, Source::Global),
        (files::Root::Thread(thread.to_owned()), PROJECT_DIR, Source::Project),
    ];
    for (root, dir, source) in roots {
        let names = match files::list(&root, dir) {
            Ok(names) => names,
            Err(error) => {
                problems.push(format!("{dir}: {error}"));
                continue;
            }
        };
        for name in names.iter().filter(|name| name.ends_with(".md")) {
            match files::read(&root, &format!("{dir}/{name}")) {
                Ok(Some(text)) => match profile::parse(name, &text, source) {
                    Ok(profile) => found.push(profile),
                    Err(error) => problems.push(format!("{dir}/{error}")),
                },
                Ok(None) => {}
                Err(error) => problems.push(format!("{dir}/{name}: {error}")),
            }
        }
    }
    (profile::merge(found), problems)
}

/// Who runs a reviewer: the first of the thread's choice, the profile, the plugin's
/// settings and the reviewed thread's own agent that names a provider. A model or
/// effort counts only where it was given for that provider, or for none.
fn agent_for(
    config: &ThreadConfig,
    profile: &Profile,
    defaults: &Defaults,
    source: &ThreadInfo,
) -> (String, Option<String>, Option<String>) {
    let layers = [
        (&config.provider, &config.model, &config.effort),
        (&profile.provider, &profile.model, &profile.effort),
        (&defaults.provider, &defaults.model, &defaults.effort),
    ];
    let provider = layers
        .iter()
        .find_map(|(provider, _, _)| (*provider).clone())
        .unwrap_or_else(|| source.provider.clone());
    let fits = |named: &Option<String>| named.as_ref().is_none_or(|named| *named == provider);
    let model =
        layers.iter().filter(|(p, _, _)| fits(p)).find_map(|(_, model, _)| (*model).clone());
    let effort =
        layers.iter().filter(|(p, _, _)| fits(p)).find_map(|(_, _, effort)| (*effort).clone());
    (provider, model, effort)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max).collect();
    format!("{kept} […]")
}

impl Reviewers {
    pub(crate) fn config_of(&self, thread: &str) -> ThreadConfig {
        load(&format!("{CONFIG}{thread}")).unwrap_or_else(|| self.defaults.thread_config())
    }

    /// Keep a thread's configuration, and give its agent the skill exactly while
    /// reviews are on.
    pub(crate) fn set_config(&mut self, thread: &str, config: &ThreadConfig) {
        let was = self.config_of(thread).enabled;
        save(&format!("{CONFIG}{thread}"), config);
        if was != config.enabled {
            if let Err(error) = threads::set_skill(thread, SKILL_NAME, config.enabled) {
                log::warn(format!("cannot switch {SKILL_NAME} in {thread}: {error}"));
            }
            if !config.enabled {
                self.disarm(thread);
            }
        }
    }

    /// Watches end with the daemon; the reviewers they were for are kept. Watch them
    /// again: one that finished meanwhile is reported at once.
    ///
    /// A round whose last steps were lost with the daemon (a reviewer that is gone,
    /// the diff that closes the round) is finished now.
    fn rewatch(&mut self) {
        if std::mem::replace(&mut self.rewatched, true) {
            return;
        }
        let Ok(keys) = kv::keys(CRITIC) else { return };
        for key in keys {
            if threads::watch(&key[CRITIC.len()..]).is_err() {
                let _ = kv::remove(&key);
            }
        }
        let Ok(keys) = kv::keys(STATE) else { return };
        for key in keys {
            let thread = &key[STATE.len()..];
            let mut state = state_of(thread);
            let Some(round) = state.round.as_mut() else { continue };
            for critic in round.critics.iter_mut().filter(|critic| !critic.done) {
                if !matches!(kv::get(&format!("{CRITIC}{}", critic.thread)), Ok(Some(_))) {
                    critic.done = true;
                    critic.problem = Some("the reviewer was lost".into());
                }
            }
            if round.critics.iter().all(|critic| critic.done) {
                save_state(thread, &state);
                self.close_round(thread);
            }
        }
    }

    /// Every reviewer of the round is done: read the diff again to finish it.
    fn close_round(&mut self, thread: &str) {
        match threads::diff(thread) {
            Ok(request) => {
                self.diffs.insert(request, DiffFor::Finish { thread: thread.to_owned() });
            }
            Err(_) => self.finish(thread, None),
        }
    }

    /// Start a review of `thread` after `delay_ms`, replacing one already waiting.
    fn arm(&mut self, thread: &str, delay_ms: u64) {
        self.disarm(thread);
        match timers::start(delay_ms) {
            Ok(timer) => {
                self.timers.insert(timer, thread.to_owned());
                self.waiting.insert(thread.to_owned(), timer);
            }
            Err(error) => log::error(format!("cannot wait to review {thread}: {error}")),
        }
    }

    fn disarm(&mut self, thread: &str) {
        if let Some(timer) = self.waiting.remove(thread) {
            timers::cancel(timer);
            self.timers.remove(&timer);
        }
    }

    /// Whether a review is waiting for the agent to stay idle.
    pub(crate) fn armed(&self, thread: &str) -> bool {
        self.waiting.contains_key(thread)
    }

    /// How an attempt to start a round ended: noted for the panel, and told to the
    /// command that asked, if one did.
    fn answer(thread: &str, token: Option<u64>, result: Result<String, String>) {
        if let Err(error) = &result {
            Self::note(thread, error.clone());
        }
        redraw(thread);
        let Some(token) = token else { return };
        let outcome = result.map(|message| {
            Outcome::CommandResult(CommandResult { message, thread: None, data: None })
        });
        if let Err(error) = completions::complete(token, outcome.as_ref().map_err(String::as_str)) {
            log::warn(format!("cannot answer the review request: {error}"));
        }
    }

    fn note(thread: &str, note: String) {
        log::info(format!("{thread}: {note}"));
        let mut state = state_of(thread);
        state.note = Some(note);
        save_state(thread, &state);
    }

    /// Ask for the diff that starts a round. What is wrong before that is an error.
    pub(crate) fn request(&mut self, thread: &str, token: Option<u64>) -> Result<(), String> {
        let config = self.config_of(thread);
        let state = state_of(thread);
        if let Some(round) = &state.round {
            return Err(format!("review round {} is still running", round.number));
        }
        if config.max_rounds > 0 && state.rounds >= config.max_rounds {
            return Err(format!(
                "the limit of {} review rounds is reached; `qcc reviewers reset` starts counting again",
                config.max_rounds
            ));
        }
        if config.profiles.is_empty() {
            return Err("no reviewer profiles are chosen for this thread".into());
        }
        let request = threads::diff(thread)?;
        self.diffs.insert(request, DiffFor::Start { thread: thread.to_owned(), token });
        redraw(thread);
        Ok(())
    }

    /// The diff is in: start the reviewers, unless there is nothing new to review.
    /// The diff is in: unless there is nothing new to review, ask which files it
    /// touches, which the reviewers' prompts list.
    fn diff_came(
        &mut self,
        thread: &str,
        token: Option<u64>,
        answer: Result<EnvironmentDiff, String>,
    ) -> Result<(), String> {
        let EnvironmentDiff { diff, .. } =
            answer.map_err(|error| format!("cannot read the changes: {error}"))?;
        self.check_new(thread, &diff)?;
        let request = threads::changes(thread)?;
        self.changes.insert(request, Pending { thread: thread.to_owned(), token, diff });
        Ok(())
    }

    /// Whether `diff` is worth a new round now.
    fn check_new(&self, thread: &str, diff: &str) -> Result<(), String> {
        if diff.trim().is_empty() {
            return Err("there are no changes to review".into());
        }
        let config = self.config_of(thread);
        let state = state_of(thread);
        // Checked again: something may have started meanwhile.
        if let Some(round) = &state.round {
            return Err(format!("review round {} is still running", round.number));
        }
        if config.max_rounds > 0 && state.rounds >= config.max_rounds {
            return Err(format!("the limit of {} review rounds is reached", config.max_rounds));
        }
        if state.fingerprint.as_deref() == Some(diff::fingerprint(diff).as_str()) {
            return Err(format!(
                "nothing changed since review round {}; no new round",
                state.rounds
            ));
        }
        Ok(())
    }

    /// The changed files are in: start the reviewers.
    fn begin(
        &mut self,
        pending: Pending,
        answer: Result<EnvironmentChanges, String>,
    ) -> Result<String, String> {
        let Pending { thread, diff, .. } = pending;
        let thread = thread.as_str();
        let changes = answer.map_err(|error| format!("cannot list the changes: {error}"))?;
        // Checked again: something may have started meanwhile.
        self.check_new(thread, &diff)?;
        let config = self.config_of(thread);
        let mut state = state_of(thread);
        let fingerprint = diff::fingerprint(&diff);
        let files: Vec<prompt::Changed> = changes
            .files
            .iter()
            .map(|file| prompt::Changed {
                path: file.path.clone(),
                kind: match file.kind {
                    ChangeKind::Added => "added",
                    ChangeKind::Modified => "modified",
                    ChangeKind::Deleted => "deleted",
                    ChangeKind::Untracked => "untracked",
                },
                additions: file.additions,
                deletions: file.deletions,
            })
            .collect();
        let read_only = read_only_providers();
        let source = threads::get(thread)?;
        let task = threads::transcript(thread).ok().and_then(|messages| {
            messages.into_iter().find(|m| m.role == threads::Role::User).map(|m| m.text)
        });
        let prints = diff::file_prints(&diff);
        let changed = (state.rounds > 0 && !state.files.is_empty())
            .then(|| diff::changed_since(&state.files, &prints));
        let number = state.rounds + 1;
        let (profiles, _) = profiles_for(thread);
        let mut problems = Vec::new();
        let mut critics = Vec::new();
        for name in &config.profiles {
            let Some(profile) = profiles.iter().find(|profile| &profile.name == name) else {
                problems.push(format!("there is no reviewer profile {name:?}"));
                continue;
            };
            let scoped = prompt::scoped_files(&files, profile, changed.as_deref());
            if scoped.is_empty() {
                continue;
            }
            let earlier = earlier_findings(thread, name);
            let review = prompt::Review {
                thread,
                cwd: &source.cwd,
                task: task.as_deref(),
                profile,
                round: number,
                base: changes.base.as_deref(),
                branch: changes.branch.as_deref(),
                files: &files,
                changed: changed.as_deref(),
                earlier: &earlier,
            };
            let (provider, model, effort) = agent_for(&config, profile, &self.defaults, &source);
            // Read-only where the agent can enforce it; otherwise the strictest mode
            // there is, in which what it tries beyond reading is asked of the user.
            let mode = if read_only.contains(&provider) {
                PermissionMode::ReadOnly
            } else {
                problems.push(asks_note(name, &provider));
                PermissionMode::Ask
            };
            let spawned = threads::spawn(&SpawnOptions {
                provider: provider.clone(),
                cwd: Some(source.cwd.clone()),
                prompt: Some(prompt::critic(&review, &scoped)),
                model: model.clone(),
                effort,
                parent: Some(thread.to_owned()),
                environment: None,
                base: None,
                permission_mode: Some(mode),
                skills_enabled: Vec::new(),
                skills_disabled: Vec::new(),
                // A service thread: the panel is where reviews are followed.
                hidden: true,
            });
            let critic = match spawned {
                Ok(critic) => critic,
                Err(error) => {
                    problems.push(format!("cannot start reviewer {name}: {error}"));
                    continue;
                }
            };
            save(&format!("{CRITIC}{}", critic.id), &thread);
            save(&format!("{REVIEWER_OF}{}", critic.id), &thread);
            if let Err(error) = threads::watch(&critic.id) {
                log::error(format!("cannot follow reviewer {}: {error}", critic.id));
            }
            critics.push(Critic {
                profile: name.clone(),
                thread: critic.id,
                provider,
                model,
                delivery: profile.delivery.unwrap_or(config.delivery),
                severity: profile.severity,
                done: false,
                findings: Vec::new(),
                problem: None,
            });
        }
        if critics.is_empty() {
            problems.insert(0, "no reviewer had anything to look at".into());
            return Err(problems.join("; "));
        }
        let mut message = format!(
            "review round {number} started: {}",
            critics
                .iter()
                .map(|critic| format!("{} in thread {}", critic.profile, critic.thread))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for problem in &problems {
            message.push_str(&format!("\nnote: {problem}"));
        }
        state.rounds = number;
        state.fingerprint = Some(fingerprint.clone());
        state.files = prints;
        state.round = Some(Round { number, fingerprint, critics });
        state.note = (!problems.is_empty()).then(|| problems.join("; "));
        save_state(thread, &state);
        Ok(message)
    }

    /// Every reviewer of the round answered and the diff is in again: leave the
    /// comments and, where asked for, tell the agent.
    fn finish(&mut self, thread: &str, now: Option<EnvironmentDiff>) {
        let mut state = state_of(thread);
        let Some(round) = state.round.take() else { return };
        let current = now.as_ref().map(|now| now.diff.as_str()).unwrap_or_default();
        let stale =
            now.as_ref().is_some_and(|now| diff::fingerprint(&now.diff) != round.fingerprint);
        let mut record = RoundRecord {
            number: round.number,
            reviewers: round.critics.iter().map(|c| c.profile.clone()).collect(),
            comments: 0,
            general: 0,
            stale,
            sent: false,
            problems: Vec::new(),
        };
        // A finding on a file the changes do not touch cannot sit on a line of them:
        // it goes along as a remark about the whole.
        let mut changed: std::collections::BTreeSet<String> = state.files.keys().cloned().collect();
        changed.extend(diff::files(current).into_iter().map(|file| file.path));
        let mut sent_comments: Vec<String> = Vec::new();
        let mut sent_remarks: Vec<(String, Finding)> = Vec::new();
        let mut new_records: Vec<ledger::Record> = Vec::new();
        for critic in &round.critics {
            if let Some(problem) = &critic.problem {
                record.problems.push(format!("{}: {problem}", critic.profile));
            }
            for finding in &critic.findings {
                let comment = match (&finding.file, finding.line) {
                    (Some(path), Some(line)) if changed.contains(path) => {
                        let comment = comments::NewComment {
                            thread: thread.to_owned(),
                            path: path.clone(),
                            line,
                            side: comments::DiffSide::New,
                            quote: diff::line_text(current, path, line),
                            text: finding.text.clone(),
                            author: comments::CommentAuthor {
                                name: critic.profile.clone(),
                                thread: Some(critic.thread.clone()),
                                severity: Some(finding.severity.as_str().to_owned()),
                                stale,
                            },
                        };
                        match comments::add(&comment) {
                            Ok(comment) => Some(comment.id),
                            Err(error) => {
                                log::warn(format!(
                                    "cannot leave a comment on {path}:{line}: {error}"
                                ));
                                None
                            }
                        }
                    }
                    _ => None,
                };
                if comment.is_some() {
                    record.comments += 1;
                } else {
                    record.general += 1;
                }
                let auto = critic.delivery == Delivery::Auto;
                match &comment {
                    Some(id) if auto => sent_comments.push(id.clone()),
                    None if auto => sent_remarks.push((critic.profile.clone(), finding.clone())),
                    _ => {}
                }
                let remark = comment.is_none();
                state.last_finding += 1;
                new_records.push(ledger::Record {
                    id: format!("f{}", state.last_finding),
                    round: round.number,
                    reviewer: critic.profile.clone(),
                    critic: critic.thread.clone(),
                    file: finding.file.clone(),
                    line: finding.line,
                    severity: finding.severity,
                    text: finding.text.clone(),
                    stale,
                    comment,
                    status: if auto && remark { Status::Sent } else { Status::Open },
                    dismissed: false,
                });
            }
        }
        let auto = round.critics.iter().any(|critic| critic.delivery == Delivery::Auto);
        if auto
            && (!sent_comments.is_empty()
                || !sent_remarks.is_empty()
                || !record.problems.is_empty())
        {
            let remarks: Vec<prompt::Remark<'_>> = sent_remarks
                .iter()
                .map(|(reviewer, finding)| prompt::Remark { reviewer, finding })
                .collect();
            let intro =
                format!("qcc reviewers finished review round {} of your changes.", round.number);
            let message = prompt::findings_message(
                &intro,
                sent_comments.len(),
                &remarks,
                stale,
                &record.problems,
            );
            match deliver(thread, &sent_comments, &message) {
                Ok(answer_in) => {
                    record.sent = true;
                    state.awaiting_response = answer_in;
                }
                Err(error) => {
                    record.problems.push(format!("cannot send the findings: {error}"));
                    // Not sent after all.
                    for finding in &mut new_records {
                        finding.status = Status::Open;
                    }
                }
            }
        }
        state.findings.extend(new_records);
        // Drawn again once saved, below.
        state.history.push(record);
        let excess = state.history.len().saturating_sub(config::HISTORY);
        state.history.drain(..excess);
        save_state(thread, &state);
        redraw(thread);
    }
}

/// Hand findings to the agent: their comments through the daemon, as the user's
/// "send to agent" does, with `text`. Returns in how many turn ends the agent's
/// answer ends: 1 when it is idle, 2 behind a running turn.
fn deliver(thread: &str, comment_ids: &[String], text: &str) -> Result<u32, String> {
    let busy = threads::get(thread).is_ok_and(|info| info.state != ThreadState::Idle);
    if comment_ids.is_empty() {
        threads::send(thread, text)?;
    } else {
        comments::send(thread, comment_ids, text)?;
    }
    Ok(if busy { 2 } else { 1 })
}

/// Tell the app that the thread's panel shows something else now.
pub(crate) fn redraw(thread: &str) {
    if let Err(error) = views::refresh(panel::VIEW, Some(thread)) {
        log::debug(format!("cannot redraw the panel of {thread}: {error}"));
    }
}

/// The thread's findings, oldest first, each with where it stands and the agent's
/// note on a fixed one.
pub(crate) fn findings_of(thread: &str) -> Vec<ledger::Standing> {
    let state = state_of(thread);
    let comments: Vec<ledger::CommentState> = comments::list(thread, true)
        .unwrap_or_default()
        .into_iter()
        .filter(|comment| comment.plugin.as_deref() == Some(PLUGIN_ID))
        .map(|comment| ledger::CommentState {
            id: comment.id,
            resolved: comment.resolved_at.is_some(),
            resolution: comment.resolution,
            sent: comment.sent_at.is_some(),
        })
        .collect();
    state
        .findings
        .into_iter()
        .map(|record| {
            let (status, note) = ledger::status(&record, &comments);
            (record, status, note)
        })
        .collect()
}

/// The findings named by `ids` (all when empty and `all` says which), or the first
/// id that names none.
fn pick(
    findings: &[ledger::Standing],
    ids: &[String],
    all: impl Fn(Status) -> bool,
) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(findings.iter().filter(|f| all(f.1)).map(|f| f.0.id.clone()).collect());
    }
    for id in ids {
        if !findings.iter().any(|f| &f.0.id == id) {
            return Err(format!(
                "this thread has no finding {id}; `qcc reviewers findings` lists them"
            ));
        }
    }
    Ok(ids.to_vec())
}

/// Send findings to the agent in one message: the open ones when `ids` is empty.
pub(crate) fn send_findings(thread: &str, ids: &[String]) -> Result<String, String> {
    let findings = findings_of(thread);
    let ids = pick(&findings, ids, |status| status == Status::Open)?;
    let chosen: Vec<&ledger::Standing> =
        findings.iter().filter(|f| ids.contains(&f.0.id)).collect();
    if let Some(done) = chosen.iter().find(|f| !f.1.pending()) {
        return Err(format!("finding {} is {}; restore it first", done.0.id, done.1.as_str()));
    }
    if chosen.is_empty() {
        return Err("there are no open findings to send".into());
    }
    let comment_ids: Vec<String> = chosen.iter().filter_map(|f| f.0.comment.clone()).collect();
    let remarks_of: Vec<Finding> = chosen
        .iter()
        .filter(|f| f.0.comment.is_none())
        .map(|f| Finding {
            file: f.0.file.clone(),
            line: f.0.line,
            severity: f.0.severity,
            text: f.0.text.clone(),
        })
        .collect();
    let remarks: Vec<prompt::Remark<'_>> = chosen
        .iter()
        .filter(|f| f.0.comment.is_none())
        .zip(&remarks_of)
        .map(|(f, finding)| prompt::Remark { reviewer: &f.0.reviewer, finding })
        .collect();
    let stale = chosen.iter().any(|f| f.0.stale);
    let message = prompt::findings_message(
        "The user asks you to deal with findings of qcc reviewers about your changes.",
        comment_ids.len(),
        &remarks,
        stale,
        &[],
    );
    let answer_in = deliver(thread, &comment_ids, &message)?;
    let mut state = state_of(thread);
    state.awaiting_response = answer_in;
    for record in state.findings.iter_mut().filter(|r| ids.contains(&r.id) && r.comment.is_none()) {
        record.status = Status::Sent;
    }
    save_state(thread, &state);
    redraw(thread);
    Ok(format!("sent {} to the agent", count_of(chosen.len(), "finding")))
}

/// Dismiss findings (`restore`: open them again).
pub(crate) fn dismiss_findings(
    thread: &str,
    ids: &[String],
    restore: bool,
) -> Result<String, String> {
    if ids.is_empty() {
        return Err("name the findings by id; `qcc reviewers findings` lists them".into());
    }
    let findings = findings_of(thread);
    let ids = pick(&findings, ids, |_| false)?;
    let mut state = state_of(thread);
    for record in state.findings.iter_mut().filter(|r| ids.contains(&r.id)) {
        match &record.comment {
            Some(comment) => {
                let note = (!restore).then(|| ledger::DISMISSED_NOTE.to_owned());
                // A deleted comment has nothing to resolve; the finding stays as it is.
                if comments::resolve(std::slice::from_ref(comment), note.as_deref(), restore)
                    .is_ok()
                {
                    record.dismissed = !restore;
                }
            }
            None => record.status = if restore { Status::Open } else { Status::Dismissed },
        }
    }
    save_state(thread, &state);
    redraw(thread);
    let done = if restore { "restored" } else { "dismissed" };
    Ok(format!("{done} {}", count_of(ids.len(), "finding")))
}

/// `1 finding`, `2 findings`.
pub(crate) fn count_of(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// Count rounds again and forget what was reviewed. A round in flight is dropped:
/// its reviewers go on, but nobody listens to them any more.
pub(crate) fn reset(thread: &str) {
    let mut state = state_of(thread);
    if let Some(round) = state.round.take() {
        for critic in round.critics {
            let _ = kv::remove(&format!("{CRITIC}{}", critic.thread));
        }
    }
    state.rounds = 0;
    state.fingerprint = None;
    state.files.clear();
    state.note = None;
    save_state(thread, &state);
}

/// The remarks a profile made in earlier rounds, as its comments hold them.
fn earlier_findings(thread: &str, profile: &str) -> Vec<prompt::Earlier> {
    let Ok(all) = comments::list(thread, true) else { return Vec::new() };
    all.into_iter()
        .filter(|comment| {
            comment.plugin.as_deref() == Some(PLUGIN_ID)
                && comment.author.as_ref().is_some_and(|author| author.name == profile)
        })
        .map(|comment| prompt::Earlier {
            location: format!("{}:{}", comment.path, comment.line),
            text: comment.text,
            resolved: comment.resolved_at.is_some(),
            resolution: comment.resolution,
        })
        .collect()
}

fn param(name: &str, label: &str, placeholder: &str) -> CommandParam {
    CommandParam {
        name: name.into(),
        label: label.into(),
        required: false,
        placeholder: Some(placeholder.into()),
        positional: false,
    }
}

/// The finding ids a command takes as words: `qcc reviewers send f1 f2`.
fn ids_param(label: &str, placeholder: &str) -> CommandParam {
    CommandParam { positional: true, ..param("ids", label, placeholder) }
}

fn ids_arg(call: &CommandCall) -> Vec<String> {
    call.args
        .iter()
        .filter(|(key, _)| key == "ids")
        .flat_map(|(_, value)| value.split([' ', ',']).map(str::trim).map(str::to_owned))
        .filter(|id| !id.is_empty())
        .collect()
}

fn flag(value: &str) -> Result<bool, String> {
    match value {
        "true" | "on" | "yes" | "1" => Ok(true),
        "false" | "off" | "no" | "0" => Ok(false),
        other => Err(format!("{other:?} is not on or off")),
    }
}

/// `None` for an empty value: back to the profile's choice.
fn optional(value: &str) -> Option<String> {
    Some(value.to_owned()).filter(|value| !value.is_empty())
}

/// Change one setting of a thread's reviews, by the name the CLI and the panel use.
pub(crate) fn apply(config: &mut ThreadConfig, key: &str, value: &str) -> Result<(), String> {
    let number =
        |value: &str| value.parse::<u32>().map_err(|_| format!("{value:?} is not a number"));
    match key {
        "enabled" => config.enabled = flag(value)?,
        "profiles" => {
            config.profiles = value
                .split(',')
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .collect();
        }
        "provider" => config.provider = optional(value),
        "model" => config.model = optional(value),
        "effort" => config.effort = optional(value),
        "delivery" => {
            config.delivery = Delivery::parse(value)
                .ok_or_else(|| format!("delivery is collect or auto, not {value:?}"))?;
        }
        "trigger" => {
            let trigger =
                Trigger::parse(value).ok_or_else(|| format!("unknown trigger {value:?}"))?;
            if !Trigger::OFFERED.contains(&trigger) {
                return Err(format!("the {value} trigger is not available yet"));
            }
            config.trigger = trigger;
        }
        "every" => config.every = number(value)?.max(1),
        "delay" => {
            config.delay_ms =
                config::parse_delay(value).ok_or_else(|| format!("{value:?} is not a delay"))?;
        }
        "max-rounds" => config.max_rounds = number(value)?,
        "review-responses" => config.review_responses = flag(value)?,
        other => return Err(format!("no setting {other:?}")),
    }
    Ok(())
}

/// A thread's reviews in a few lines, for the CLI.
fn describe(config: &ThreadConfig, state: &Progress) -> String {
    let or_profile =
        |value: &Option<String>| value.clone().unwrap_or_else(|| "from the profile".into());
    let limit =
        if config.max_rounds == 0 { "no limit".to_owned() } else { config.max_rounds.to_string() };
    let mut text = format!(
        "reviews {}; profiles: {}; agent: {}, model: {}, effort: {}; delivery: {}; trigger: {}",
        if config.enabled { "on" } else { "off" },
        if config.profiles.is_empty() { "none".to_owned() } else { config.profiles.join(", ") },
        or_profile(&config.provider),
        or_profile(&config.model),
        or_profile(&config.effort),
        config.delivery.as_str(),
        config.trigger.as_str(),
    );
    match config.trigger {
        Trigger::Finished => {
            text.push_str(&format!(" after {}", config::format_delay(config.delay_ms)))
        }
        Trigger::EveryTurns => text.push_str(&format!(" {}", config.every)),
        _ => {}
    }
    text.push_str(&format!("; rounds: {} of {limit}", state.rounds));
    if let Some(round) = &state.round {
        text.push_str(&format!("; round {} is running", round.number));
    }
    text
}

impl Commands for Reviewers {
    fn commands(&self) -> Vec<CommandInfo> {
        vec![
            CommandInfo {
                id: "request".into(),
                title: "Review this thread now".into(),
                description: "Start a review round of this thread's changes now, without \
                              waiting for its trigger. An agent runs it when it is done."
                    .into(),
                thread: true,
                params: Vec::new(),
            },
            CommandInfo {
                id: "configure".into(),
                title: "Set up this thread's reviewers".into(),
                description: "Change how this thread is reviewed; with nothing given, say how \
                              it is."
                    .into(),
                thread: true,
                params: vec![
                    param("enabled", "Review on its own", "on or off"),
                    param("profiles", "Profiles", "general,security"),
                    param("provider", "Agent", "empty: the profile's"),
                    param("model", "Model", "empty: the profile's"),
                    param("effort", "Effort", "empty: the profile's"),
                    param("delivery", "Delivery", "collect or auto"),
                    param("trigger", "Trigger", "finished, every-turn, every-turns, manual"),
                    param("every", "Every so many turns", "3"),
                    param("delay", "Wait after the agent is done", "30s"),
                    param("max-rounds", "Rounds at most", "3; 0 for no limit"),
                    param("review-responses", "Review the answers to findings", "on or off"),
                ],
            },
            CommandInfo {
                id: "findings".into(),
                title: "List this thread's review findings".into(),
                description: "Every finding of every round, with where it stands: open, sent \
                              to the agent, fixed or dismissed."
                    .into(),
                thread: true,
                params: Vec::new(),
            },
            CommandInfo {
                id: "send".into(),
                title: "Send review findings to the agent".into(),
                description: "Hand findings to the agent in one message: those named, or every \
                              open one."
                    .into(),
                thread: true,
                params: vec![ids_param("Findings", "f1 f2; empty: every open one")],
            },
            CommandInfo {
                id: "dismiss".into(),
                title: "Dismiss review findings".into(),
                description: "Mark findings as needing nothing; their comments are resolved."
                    .into(),
                thread: true,
                params: vec![CommandParam { required: true, ..ids_param("Findings", "f1 f2") }],
            },
            CommandInfo {
                id: "restore".into(),
                title: "Restore dismissed review findings".into(),
                description: "Open dismissed or fixed findings again.".into(),
                thread: true,
                params: vec![CommandParam { required: true, ..ids_param("Findings", "f1 f2") }],
            },
            CommandInfo {
                id: "reset".into(),
                title: "Count review rounds again".into(),
                description: "Forget the rounds this thread has had and what they reviewed, \
                              so reviews start again. A round still running is dropped."
                    .into(),
                thread: true,
                params: Vec::new(),
            },
            CommandInfo {
                id: "profiles".into(),
                title: "List reviewer profiles".into(),
                description: "The reviewer profiles this thread can use, and where each is from."
                    .into(),
                thread: true,
                params: Vec::new(),
            },
        ]
    }

    fn run(&mut self, call: CommandCall) -> Result<Reply<CommandResult>, String> {
        self.rewatch();
        let thread = call.thread.clone().ok_or("name a thread")?;
        let done =
            |message: String| Ok(Reply::Now(CommandResult { message, thread: None, data: None }));
        match call.command.as_str() {
            "request" => {
                self.next_token += 1;
                let token = self.next_token;
                self.request(&thread, Some(token))?;
                Ok(Reply::Later(token))
            }
            "configure" => {
                let mut config = self.config_of(&thread);
                for (key, value) in &call.args {
                    apply(&mut config, key, value.trim())?;
                }
                if !call.args.is_empty() {
                    self.set_config(&thread, &config);
                }
                done(describe(&config, &state_of(&thread)))
            }
            "findings" => {
                let findings = findings_of(&thread);
                let data: Vec<serde_json::Value> = findings
                    .iter()
                    .map(|(record, status, note)| {
                        serde_json::json!({
                            "id": record.id,
                            "round": record.round,
                            "reviewer": record.reviewer,
                            "reviewer_thread": record.critic,
                            "file": record.file,
                            "line": record.line,
                            "severity": record.severity,
                            "text": record.text,
                            "stale": record.stale,
                            "comment": record.comment,
                            "status": status.as_str(),
                            "note": note,
                        })
                    })
                    .collect();
                let mut lines: Vec<String> = findings
                    .iter()
                    .map(|(record, status, note)| {
                        let at = record.location().map(|at| format!(" {at}")).unwrap_or_default();
                        let stale = if record.stale { " (stale)" } else { "" };
                        let note = note.as_deref().map(|n| format!(" — {n}")).unwrap_or_default();
                        format!(
                            "{}  {:<9} {:<8} {}{at}{stale}: {}{note}",
                            record.id,
                            status.as_str(),
                            record.severity.as_str(),
                            record.reviewer,
                            record.text
                        )
                    })
                    .collect();
                if lines.is_empty() {
                    lines.push("no findings yet".into());
                }
                let data = serde_json::to_string(&data).expect("findings serialize");
                Ok(Reply::Now(CommandResult {
                    message: lines.join("\n"),
                    thread: None,
                    data: Some(data),
                }))
            }
            "send" => done(send_findings(&thread, &ids_arg(&call))?),
            "dismiss" => done(dismiss_findings(&thread, &ids_arg(&call), false)?),
            "restore" => done(dismiss_findings(&thread, &ids_arg(&call), true)?),
            "reset" => {
                reset(&thread);
                done("review rounds are counted again from none".into())
            }
            "profiles" => {
                let (profiles, problems) = profiles_for(&thread);
                let mut lines: Vec<String> = profiles
                    .iter()
                    .map(|p| {
                        format!(
                            "{} ({}): {}",
                            p.name,
                            p.source,
                            p.description.as_deref().unwrap_or("no description")
                        )
                    })
                    .collect();
                lines.extend(problems.into_iter().map(|problem| format!("cannot read {problem}")));
                done(lines.join("\n"))
            }
            other => Err(format!("unknown command {other:?}")),
        }
    }
}

impl Plugin for Reviewers {
    fn commands(&mut self) -> Option<&mut dyn Commands> {
        Some(self)
    }

    fn ui(&mut self) -> Option<&mut dyn Ui> {
        Some(self)
    }

    fn skills(&self) -> Vec<SkillInfo> {
        vec![SkillInfo {
            name: SKILL_NAME.into(),
            description: "Reviewers check your work in this thread. Read it to learn how to \
                          ask for a review when you are done and how to answer the findings."
                .into(),
            body: SKILL_BODY.into(),
            // Turned on in a thread when the user turns its reviews on.
            on_request: true,
        }]
    }

    fn settings(&self) -> Vec<SettingField> {
        let text = |id: &str, label: &str, description: &str, default: &str| SettingField {
            id: id.into(),
            label: label.into(),
            description: Some(description.into()),
            kind: SettingKind::Text,
            default: Some(default.into()),
        };
        vec![
            text(
                "profiles",
                "Profiles",
                "The reviewer profiles a thread starts with, separated by commas.",
                "general",
            ),
            SettingField {
                id: "provider".into(),
                label: "Agent".into(),
                description: Some(
                    "Who reviews when neither the thread nor the profile says. Empty: the \
                     reviewed thread's own agent."
                        .into(),
                ),
                kind: SettingKind::Provider,
                default: None,
            },
            SettingField {
                id: "model".into(),
                label: "Model".into(),
                description: None,
                kind: SettingKind::Model("provider".into()),
                default: None,
            },
            SettingField {
                id: "effort".into(),
                label: "Effort".into(),
                description: None,
                kind: SettingKind::Effort(("provider".into(), "model".into())),
                default: None,
            },
            SettingField {
                id: "delivery".into(),
                label: "Delivery".into(),
                description: Some(
                    "collect: findings stay comments for you to send; auto: they are also sent \
                     to the agent."
                        .into(),
                ),
                kind: SettingKind::Choice(vec!["collect".into(), "auto".into()]),
                default: Some("collect".into()),
            },
            SettingField {
                id: "trigger".into(),
                label: "Trigger".into(),
                description: None,
                kind: SettingKind::Choice(
                    Trigger::OFFERED.iter().map(|trigger| trigger.as_str().to_owned()).collect(),
                ),
                default: Some("finished".into()),
            },
            text(
                "delay",
                "Delay",
                "How long the agent must stay idle before a review: 30s, 2m.",
                "30s",
            ),
            text("max_rounds", "Rounds at most", "Per thread; 0 for no limit.", "3"),
        ]
    }

    fn on_settings(&mut self, values: Vec<(String, String)>) {
        self.defaults = Defaults::from_settings(&values);
    }

    fn on_turn_ended(&mut self, ended: TurnEnded) {
        self.rewatch();
        let thread = ended.thread.id;
        // A reviewer's own turns, and threads never set up, are not reviewed.
        let Some(config) = load::<ThreadConfig>(&format!("{CONFIG}{thread}")) else { return };
        if !config.enabled || matches!(kv::get(&format!("{CRITIC}{thread}")), Ok(Some(_))) {
            return;
        }
        let mut state = state_of(&thread);
        let answering = match state.awaiting_response {
            0 => false,
            left => {
                state.awaiting_response = left - 1;
                left == 1
            }
        };
        if ended.status != TurnStatus::Completed {
            save_state(&thread, &state);
            self.disarm(&thread);
            return;
        }
        state.turns += 1;
        let turns = state.turns;
        save_state(&thread, &state);
        if answering && !config.review_responses {
            Self::note(
                &thread,
                "the agent answered the findings; that is not reviewed again".into(),
            );
            return;
        }
        match config.trigger {
            Trigger::Finished => self.arm(&thread, config.delay_ms),
            Trigger::EveryTurn => self.arm(&thread, 0),
            Trigger::EveryTurns if turns.is_multiple_of(config.every.max(1)) => {
                self.arm(&thread, 0)
            }
            _ => {}
        }
    }

    fn on_timer(&mut self, timer: u64) {
        let Some(thread) = self.timers.remove(&timer) else { return };
        if self.waiting.get(&thread) == Some(&timer) {
            self.waiting.remove(&thread);
        }
        // A turn that started meanwhile ends again and waits anew.
        let Ok(info) = threads::get(&thread) else { return };
        if info.state != ThreadState::Idle || info.last_turn != Some(TurnStatus::Completed) {
            return;
        }
        if let Err(error) = self.request(&thread, None) {
            Self::note(&thread, error);
        }
    }

    fn on_provider_models(&mut self, answer: ProviderModels) {
        self.models_came(answer);
    }

    fn on_comments_changed(&mut self, thread: String) {
        // The agent resolved one, the user sent or reopened one: their findings moved.
        if !state_of(&thread).findings.is_empty() {
            redraw(&thread);
        }
    }

    fn on_thread_changes(&mut self, answer: ThreadChanges) {
        let Some(pending) = self.changes.remove(&answer.request) else { return };
        let (thread, token) = (pending.thread.clone(), pending.token);
        let result = self.begin(pending, answer.answer);
        Self::answer(&thread, token, result);
    }

    fn on_thread_diff(&mut self, answer: ThreadDiff) {
        match self.diffs.remove(&answer.request) {
            Some(DiffFor::Start { thread, token }) => {
                if let Err(error) = self.diff_came(&thread, token, answer.answer) {
                    Self::answer(&thread, token, Err(error));
                }
            }
            Some(DiffFor::Finish { thread }) => self.finish(&thread, answer.answer.ok()),
            None => {}
        }
    }

    fn on_thread_settled(&mut self, settled: SettledThread) {
        self.rewatch();
        let reviewer = settled.thread;
        let key = format!("{CRITIC}{}", reviewer.id);
        let Some(thread) = load::<String>(&key) else { return };
        let _ = kv::remove(&key);
        let mut state = state_of(&thread);
        let Some(round) = state.round.as_mut() else { return };
        let Some(critic) = round.critics.iter_mut().find(|c| c.thread == reviewer.id) else {
            return;
        };
        let completed = reviewer.state == ThreadState::Idle
            && reviewer.last_turn == Some(TurnStatus::Completed);
        match settled.last_message {
            Some(reply) if completed => match findings::parse(&reply) {
                Report::Findings(found) => {
                    critic.findings =
                        found.into_iter().filter(|f| f.severity >= critic.severity).collect();
                }
                // Its words are all there is: they go along as one remark.
                Report::Unstructured => {
                    critic.findings = vec![Finding {
                        file: None,
                        line: None,
                        severity: critic.severity,
                        text: clip(reply.trim(), MAX_LOOSE_REPLY),
                    }];
                }
            },
            _ => critic.problem = Some("the reviewer stopped without an answer".into()),
        }
        critic.done = true;
        let all_done = round.critics.iter().all(|critic| critic.done);
        save_state(&thread, &state);
        redraw(&thread);
        if all_done {
            self.close_round(&thread);
        }
    }
}

qcc_plugin_sdk::export_plugin!(Reviewers);

#[cfg(test)]
mod tests {
    use super::*;

    fn info(provider: &str) -> ThreadInfo {
        ThreadInfo {
            id: "thr_1".into(),
            provider: provider.into(),
            cwd: "/w".into(),
            parent: None,
            title: None,
            hidden: false,
            model: Some("its-model".into()),
            effort: None,
            environment: None,
            state: ThreadState::Idle,
            last_turn: Some(TurnStatus::Completed),
        }
    }

    fn profile(front: &str) -> Profile {
        profile::parse("p.md", &format!("---\n{front}\n---\nbody"), Source::Project).unwrap()
    }

    #[test]
    fn the_agent_comes_from_the_first_layer_that_names_one() {
        let defaults = Defaults::default();
        let mut config = ThreadConfig::default();
        let plain = profile("name: p");
        // Nothing names one: the reviewed thread's agent, with no model of its own.
        assert_eq!(
            agent_for(&config, &plain, &defaults, &info("codex")),
            ("codex".into(), None, None)
        );
        // The profile's agent and model.
        let named = profile("provider: claude-code\nmodel: haiku\neffort: low");
        let picked = agent_for(&config, &named, &defaults, &info("codex"));
        assert_eq!(picked, ("claude-code".into(), Some("haiku".into()), Some("low".into())));
        // A thread's model without an agent applies to the profile's agent.
        config.model = Some("sonnet".into());
        let picked = agent_for(&config, &named, &defaults, &info("codex"));
        assert_eq!(picked, ("claude-code".into(), Some("sonnet".into()), Some("low".into())));
        // A thread's agent drops the profile's model, which was for another agent.
        config.provider = Some("codex".into());
        config.model = None;
        assert_eq!(
            agent_for(&config, &named, &defaults, &info("echo")),
            ("codex".into(), None, None)
        );
    }

    #[test]
    fn settings_are_changed_by_name() {
        let mut config = ThreadConfig::default();
        apply(&mut config, "enabled", "on").unwrap();
        apply(&mut config, "profiles", "general, security").unwrap();
        apply(&mut config, "delivery", "auto").unwrap();
        apply(&mut config, "trigger", "every-turns").unwrap();
        apply(&mut config, "every", "0").unwrap();
        apply(&mut config, "delay", "250ms").unwrap();
        apply(&mut config, "max-rounds", "0").unwrap();
        apply(&mut config, "model", "").unwrap();
        assert!(config.enabled);
        assert_eq!(config.profiles, ["general", "security"]);
        assert_eq!((config.delivery, config.trigger), (Delivery::Auto, Trigger::EveryTurns));
        assert_eq!((config.every, config.delay_ms, config.max_rounds), (1, 250, 0));
        assert!(apply(&mut config, "trigger", "commit").unwrap_err().contains("not available yet"));
        assert!(apply(&mut config, "delivery", "x").is_err());
        assert!(apply(&mut config, "colour", "x").is_err());
        let text = describe(&config, &Progress::default());
        assert!(text.contains("reviews on") && text.contains("rounds: 0 of no limit"), "{text}");
    }
}
