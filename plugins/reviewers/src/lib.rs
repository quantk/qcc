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
mod panel;
pub mod profile;
pub mod prompt;

use std::collections::HashMap;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::threads::SpawnOptions;

use crate::config::{Critic, Defaults, Progress, Round, RoundRecord, ThreadConfig, Trigger};
use crate::findings::{Finding, Report};
use crate::profile::{Delivery, Profile, Source};

const SKILL_NAME: &str = "qcc-reviewers";
const SKILL_BODY: &str = include_str!("../SKILL.md");

/// `config/<thread>`: the reviews the user set up for a thread.
const CONFIG: &str = "config/";
/// `state/<thread>`: rounds, fingerprints and the round in flight.
const STATE: &str = "state/";
/// `critic/<reviewer thread>`: the thread it reviews, while it works.
const CRITIC: &str = "critic/";

/// Where profiles live besides the built-in ones.
const GLOBAL_DIR: &str = "profiles";
const PROJECT_DIR: &str = ".qcc/reviewers";
/// The longest reply of a reviewer that did not follow the format, kept as one finding.
const MAX_LOOSE_REPLY: usize = 4_000;

/// Why a diff was asked for.
enum DiffFor {
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
    next_token: u64,
    /// Agents' models, for the panel.
    models: HashMap<String, panel::Models>,
    /// The agent each `threads.models` request is about.
    model_requests: HashMap<u64, String>,
}

fn load<T: serde::de::DeserializeOwned>(key: &str) -> Option<T> {
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
        Ok(())
    }

    /// The diff is in: start the reviewers, unless there is nothing new to review.
    fn begin(
        &mut self,
        thread: &str,
        answer: Result<EnvironmentDiff, String>,
    ) -> Result<String, String> {
        let EnvironmentDiff { diff, truncated } =
            answer.map_err(|error| format!("cannot read the changes: {error}"))?;
        if diff.trim().is_empty() {
            return Err("there are no changes to review".into());
        }
        let config = self.config_of(thread);
        let mut state = state_of(thread);
        // Checked again: something may have started meanwhile.
        if let Some(round) = &state.round {
            return Err(format!("review round {} is still running", round.number));
        }
        if config.max_rounds > 0 && state.rounds >= config.max_rounds {
            return Err(format!("the limit of {} review rounds is reached", config.max_rounds));
        }
        let fingerprint = diff::fingerprint(&diff);
        if state.fingerprint.as_deref() == Some(fingerprint.as_str()) {
            return Err(format!(
                "nothing changed since review round {}; no new round",
                state.rounds
            ));
        }
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
            let Some(scoped) = prompt::scoped_diff(&diff, profile, changed.as_deref()) else {
                continue;
            };
            let earlier = earlier_findings(thread, name);
            let review = prompt::Review {
                thread,
                cwd: &source.cwd,
                task: task.as_deref(),
                profile,
                round: number,
                diff: &diff,
                truncated,
                changed: changed.as_deref(),
                earlier: &earlier,
            };
            let (provider, model, effort) = agent_for(&config, profile, &self.defaults, &source);
            let spawned = threads::spawn(&SpawnOptions {
                provider: provider.clone(),
                cwd: Some(source.cwd.clone()),
                prompt: Some(prompt::critic(&review, &scoped)),
                model: model.clone(),
                effort,
                parent: Some(thread.to_owned()),
                environment: None,
                base: None,
                // Reviewers look; anything else they want is asked of the user.
                permission_mode: Some(PermissionMode::Ask),
                skills_enabled: Vec::new(),
                skills_disabled: Vec::new(),
                hidden: false,
            });
            let critic = match spawned {
                Ok(critic) => critic,
                Err(error) => {
                    problems.push(format!("cannot start reviewer {name}: {error}"));
                    continue;
                }
            };
            save(&format!("{CRITIC}{}", critic.id), &thread);
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
        let message = format!(
            "review round {number} started: {}",
            critics
                .iter()
                .map(|critic| format!("{} in thread {}", critic.profile, critic.thread))
                .collect::<Vec<_>>()
                .join(", ")
        );
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
        let mut sent: Vec<(String, Finding, Option<String>)> = Vec::new();
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
                if critic.delivery == Delivery::Auto {
                    sent.push((critic.profile.clone(), finding.clone(), comment));
                }
            }
        }
        let auto = round.critics.iter().any(|critic| critic.delivery == Delivery::Auto);
        if auto && (!sent.is_empty() || !record.problems.is_empty()) {
            let items: Vec<prompt::Sent<'_>> = sent
                .iter()
                .map(|(reviewer, finding, comment)| prompt::Sent {
                    reviewer,
                    finding,
                    comment: comment.as_deref(),
                })
                .collect();
            let message = prompt::summary(round.number, &items, stale, &record.problems);
            // The agent's next turn answers it: the one after the running one, if any.
            let busy = threads::get(thread).is_ok_and(|info| info.state != ThreadState::Idle);
            match threads::send(thread, &message) {
                Ok(()) => {
                    record.sent = true;
                    state.awaiting_response = if busy { 2 } else { 1 };
                }
                Err(error) => {
                    record.problems.push(format!("cannot send the findings: {error}"));
                }
            }
        }
        state.history.push(record);
        let excess = state.history.len().saturating_sub(config::HISTORY);
        state.history.drain(..excess);
        save_state(thread, &state);
    }
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
            comment.plugin.as_deref() == Some("reviewers")
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
    }
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
        let done = |message: String| Ok(Reply::Now(CommandResult { message, thread: None }));
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

    fn on_thread_diff(&mut self, answer: ThreadDiff) {
        match self.diffs.remove(&answer.request) {
            Some(DiffFor::Start { thread, token }) => {
                let result = self.begin(&thread, answer.answer);
                if let Err(error) = &result {
                    Self::note(&thread, error.clone());
                }
                if let Some(token) = token {
                    let outcome = result.map(|message| {
                        Outcome::CommandResult(CommandResult { message, thread: None })
                    });
                    if let Err(error) =
                        completions::complete(token, outcome.as_ref().map_err(String::as_str))
                    {
                        log::warn(format!("cannot answer the review request: {error}"));
                    }
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
