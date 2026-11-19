//! The thread's reviewer settings, in the panel beside it.

use std::collections::BTreeSet;

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::ui::{self, Node, Tone};

use crate::config::{self, Critic, ThreadConfig, Trigger};
use crate::ledger::{self, Grouping, Status};
use crate::profile::Delivery;
use crate::{
    Reviewers, apply, dismiss_findings, findings_of, profiles_for, send_findings, state_of,
};

pub(crate) const VIEW: &str = "reviewers";

const DELAYS: [u64; 6] = [5_000, 15_000, 30_000, 60_000, 120_000, 300_000];
const ROUNDS: [u32; 6] = [1, 2, 3, 5, 10, 0];
const EVERY: [u32; 4] = [2, 3, 5, 10];

/// How a reviewer of the running round is doing.
fn critic_progress(critic: &Critic) -> (String, Tone) {
    if let Some(problem) = &critic.problem {
        return (format!("error: {problem}"), Tone::Danger);
    }
    if critic.done {
        return (
            format!("done, {}", crate::count_of(critic.findings.len(), "finding")),
            Tone::Success,
        );
    }
    match threads::get(&critic.thread).map(|info| info.state) {
        Ok(ThreadState::Waiting) => ("waiting for your approval".to_owned(), Tone::Warning),
        Ok(ThreadState::Failed) => ("failed".to_owned(), Tone::Danger),
        Ok(ThreadState::Idle) => ("finishing…".to_owned(), Tone::Muted),
        Ok(ThreadState::Working) => ("looking…".to_owned(), Tone::Muted),
        Ok(ThreadState::Archived) | Err(_) => ("gone".to_owned(), Tone::Danger),
    }
}

fn severity_tone(severity: crate::findings::Severity) -> Tone {
    use crate::findings::Severity;
    match severity {
        Severity::Critical | Severity::High => Tone::Danger,
        Severity::Medium => Tone::Warning,
        Severity::Low | Severity::Info => Tone::Muted,
    }
}

/// One finding: what, where, how much it matters, where it stands, what to do.
fn finding_node(
    thread: &str,
    record: &ledger::Record,
    status: Status,
    note: Option<&str>,
    picked: &BTreeSet<String>,
) -> Node {
    let mut head = Vec::new();
    if status.pending() {
        head.push(ui::switch("", picked.contains(&record.id), format!("pick:{}", record.id)));
    }
    head.push(ui::badge(record.severity.as_str()).tone(severity_tone(record.severity)));
    let status_tone = match status {
        Status::Open => Tone::Warning,
        Status::Sent => Tone::Muted,
        Status::Fixed => Tone::Success,
        Status::Dismissed => Tone::Muted,
    };
    head.push(ui::badge(status.label()).tone(status_tone));
    match (&record.file, record.line) {
        (Some(file), line) if record.comment.is_some() => {
            let at = record.location().unwrap_or_default();
            head.push(ui::link(at, thread.to_owned(), Some(file), line));
        }
        (Some(_), _) => head.push(ui::text(record.location().unwrap_or_default())),
        (None, _) => head.push(ui::text("about the whole change").tone(Tone::Muted)),
    }
    if record.stale {
        head.push(ui::badge("stale").tone(Tone::Warning));
    }
    head.push(
        ui::text(format!("{} · round {} · {}", record.reviewer, record.round, record.id))
            .tone(Tone::Muted),
    );
    let mut body = vec![ui::row(head), ui::markdown(record.text.clone())];
    if let Some(note) = note {
        body.push(ui::text(format!("Agent: {note}")).tone(Tone::Success));
    }
    let mut actions = Vec::new();
    if status == Status::Open {
        actions.push(ui::button("Send", "send").value(record.id.clone()));
    }
    if status.pending() {
        actions.push(ui::button("Dismiss", "dismiss").value(record.id.clone()));
    } else {
        actions.push(ui::button("Restore", "restore").value(record.id.clone()));
    }
    actions.push(ui::link("reasoning", record.critic.clone(), None, None));
    body.push(ui::row(actions));
    ui::stack(body)
}

/// `1 comment`, `2 comments`.
fn count(n: u32, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// What the panel knows of an agent's models.
pub(crate) enum Models {
    /// Asked for; these threads' panels are drawn again when they come.
    Loading(BTreeSet<String>),
    Ready(Vec<ModelInfo>),
    Failed(String),
}

/// The model a choice of `model` stands for: that one, or the agent's default.
fn chosen_model<'a>(models: &'a [ModelInfo], model: Option<&str>) -> Option<&'a ModelInfo> {
    match model {
        Some(id) => models.iter().find(|m| m.id == id),
        None => models.iter().find(|m| m.is_default).or(models.first()),
    }
}

/// `(value, label)` options with "as the profile says" first, and the current value
/// kept even when the list does not have it.
fn options(
    choices: impl IntoIterator<Item = (String, String)>,
    current: Option<&str>,
) -> Vec<(String, String)> {
    let mut options = vec![(String::new(), "As the profile says".to_owned())];
    options.extend(choices);
    if let Some(current) = current
        && !options.iter().any(|(value, _)| value == current)
    {
        options.push((current.to_owned(), current.to_owned()));
    }
    options
}

/// Model and effort as lines of text, for an agent that does not list its models.
fn typed_agent(config: &ThreadConfig, nodes: &mut Vec<Node>) {
    nodes.push(
        ui::input("Model (empty: the profile's)", "Set model", "model")
            .value(config.model.clone().unwrap_or_default()),
    );
    nodes.push(
        ui::input("Effort (empty: the profile's)", "Set effort", "effort")
            .value(config.effort.clone().unwrap_or_default()),
    );
}

impl Reviewers {
    /// The models of `provider` as far as known, asking the agent the first time;
    /// the panel of `thread` is drawn again once they are in.
    fn models_of(&mut self, provider: &str, thread: &str, refresh: bool) -> &Models {
        let ask = refresh || !self.models.contains_key(provider);
        if ask {
            let state = match threads::models(provider, refresh) {
                Ok(request) => {
                    self.model_requests.insert(request, provider.to_owned());
                    Models::Loading(BTreeSet::new())
                }
                Err(error) => Models::Failed(error),
            };
            self.models.insert(provider.to_owned(), state);
        }
        let models = self.models.get_mut(provider).expect("inserted above");
        if let Models::Loading(waiting) = models {
            waiting.insert(thread.to_owned());
        }
        models
    }

    /// The models of an agent came: keep them and draw the panels that wait.
    pub(crate) fn models_came(&mut self, answer: ProviderModels) {
        if self.model_requests.remove(&answer.request).is_none() {
            return;
        }
        let state = match answer.answer {
            Ok(models) => Models::Ready(models),
            Err(error) => Models::Failed(error),
        };
        if let Some(Models::Loading(waiting)) = self.models.insert(answer.provider, state) {
            for thread in waiting {
                if let Err(error) = views::refresh(VIEW, Some(&thread)) {
                    log::warn(format!("cannot redraw the panel of {thread}: {error}"));
                }
            }
        }
    }

    /// The agent, model and effort of the thread's reviewers.
    fn agent_fields(&mut self, thread: &str, config: &ThreadConfig, nodes: &mut Vec<Node>) {
        let mut agents: Vec<(String, String)> = Vec::new();
        if let Ok(providers) = threads::providers() {
            agents.extend(providers.into_iter().map(|p| (p.id, p.name)));
        }
        nodes.push(ui::select(
            "Agent",
            options(agents, config.provider.as_deref()),
            config.provider.clone().unwrap_or_default(),
            "provider",
        ));
        let Some(provider) = config.provider.clone() else {
            if config.model.is_some() || config.effort.is_some() {
                typed_agent(config, nodes);
            } else {
                nodes.push(
                    ui::text(
                        "Each profile's agent, model and effort. Pick an agent to choose them.",
                    )
                    .tone(Tone::Muted),
                );
            }
            return;
        };
        match self.models_of(&provider, thread, false) {
            Models::Loading(_) => {
                nodes
                    .push(ui::text(format!("Loading the models of {provider}…")).tone(Tone::Muted));
            }
            Models::Ready(models) if !models.is_empty() => {
                let choices = models.iter().map(|m| {
                    let label =
                        if m.is_default { format!("{} (default)", m.name) } else { m.name.clone() };
                    (m.id.clone(), label)
                });
                nodes.push(ui::select(
                    "Model",
                    options(choices, config.model.as_deref()),
                    config.model.clone().unwrap_or_default(),
                    "model",
                ));
                let efforts = chosen_model(models, config.model.as_deref())
                    .map(|model| model.efforts.clone())
                    .unwrap_or_default();
                if !efforts.is_empty() || config.effort.is_some() {
                    nodes.push(ui::select(
                        "Effort",
                        options(
                            efforts.into_iter().map(|e| (e.clone(), e)),
                            config.effort.as_deref(),
                        ),
                        config.effort.clone().unwrap_or_default(),
                        "effort",
                    ));
                }
            }
            Models::Ready(_) => {
                nodes.push(
                    ui::text(format!("{provider} does not list its models.")).tone(Tone::Muted),
                );
                typed_agent(config, nodes);
            }
            Models::Failed(error) => {
                let note = format!("Cannot list the models of {provider}: {error}");
                nodes.push(ui::text(note).tone(Tone::Warning));
                typed_agent(config, nodes);
                nodes.push(ui::button("Ask for the models again", "models-refresh"));
            }
        }
    }

    fn panel(&mut self, thread: &str) -> Node {
        // A reviewer's own thread: what it reviewed, and the way back.
        if let Some(reviewed) = crate::load::<String>(&format!("{}{thread}", crate::REVIEWER_OF)) {
            return ui::stack([
                ui::heading("Reviewers"),
                ui::text("This thread is a reviewer: what it read and thought on the way to its findings.")
                    .tone(Tone::Muted),
                ui::link("Back to the reviewed thread", reviewed, None, None),
            ]);
        }
        let config = self.config_of(thread);
        let state = state_of(thread);
        let (profiles, problems) = profiles_for(thread);
        let mut nodes = vec![ui::heading("Reviewers")];

        // The review first: start one, and how the one running gets on.
        let limit = if config.max_rounds == 0 {
            "no limit".to_owned()
        } else {
            config.max_rounds.to_string()
        };
        let mut head = vec![
            ui::button("Run a review", "review").primary(),
            ui::badge(format!("{} of {limit} rounds", state.rounds)),
        ];
        if state.round.is_none() && self.starting(thread) {
            head.push(ui::badge("reading the changes…"));
        } else if state.round.is_none() && self.armed(thread) {
            head.push(ui::badge("next: once the agent stays idle"));
        }
        nodes.push(ui::row(head));
        if let Some(note) = &state.note {
            nodes.push(ui::text(note.clone()).tone(Tone::Muted));
        }
        if let Some(round) = &state.round {
            nodes.push(ui::text(format!("Round {}", round.number)).tone(Tone::Muted));
            for critic in &round.critics {
                let (label, tone) = critic_progress(critic);
                nodes.push(ui::row([
                    ui::badge(label).tone(tone),
                    ui::text(critic.profile.clone()),
                    ui::link("reasoning", critic.thread.clone(), None, None),
                ]));
            }
        }

        self.findings_section(thread, &config, &mut nodes);

        nodes.push(ui::divider());
        nodes.push(ui::heading("Settings"));
        nodes.push(ui::switch("Review this thread on its own", config.enabled, "enabled"));
        nodes.push(ui::text("Profiles").tone(Tone::Muted));
        for profile in &profiles {
            let chosen = config.profiles.contains(&profile.name);
            let label = match &profile.description {
                Some(description) => {
                    format!("{} — {description} ({})", profile.name, profile.source)
                }
                None => format!("{} ({})", profile.name, profile.source),
            };
            nodes.push(ui::switch(label, chosen, format!("profile:{}", profile.name)));
        }
        for missing in
            config.profiles.iter().filter(|name| !profiles.iter().any(|p| &p.name == *name))
        {
            nodes.push(ui::text(format!("No profile {missing:?} is found")).tone(Tone::Warning));
        }
        for problem in &problems {
            nodes.push(ui::text(format!("Cannot read {problem}")).tone(Tone::Warning));
        }

        self.agent_fields(thread, &config, &mut nodes);
        // Reviewers whose agent cannot run read-only may stop to ask.
        if let Ok(source) = threads::get(thread) {
            let read_only = crate::read_only_providers();
            for profile in profiles.iter().filter(|p| config.profiles.contains(&p.name)) {
                let (provider, _, _) = crate::agent_for(&config, profile, &self.defaults, &source);
                if !read_only.contains(&provider) {
                    let note = crate::asks_note(&profile.name, &provider);
                    nodes.push(ui::text(format!("{note}.")).tone(Tone::Warning));
                }
            }
        }

        nodes.push(ui::select(
            "Findings",
            [
                (Delivery::Collect.as_str(), "Comments for me to send"),
                (Delivery::Auto.as_str(), "Comments, sent to the agent"),
            ],
            config.delivery.as_str(),
            "delivery",
        ));
        nodes.push(ui::select(
            "When",
            Trigger::OFFERED.map(|trigger| (trigger.as_str(), trigger.label())),
            config.trigger.as_str(),
            "trigger",
        ));
        match config.trigger {
            Trigger::Finished => {
                let mut delays: Vec<u64> = DELAYS.to_vec();
                if !delays.contains(&config.delay_ms) {
                    delays.push(config.delay_ms);
                    delays.sort_unstable();
                }
                nodes.push(ui::select(
                    "After the agent is idle for",
                    delays.iter().map(|ms| (ms.to_string(), config::format_delay(*ms))),
                    config.delay_ms.to_string(),
                    "delay-ms",
                ));
            }
            Trigger::EveryTurns => {
                nodes.push(ui::select(
                    "Every",
                    EVERY.map(|n| (n.to_string(), format!("{n} turns"))),
                    config.every.to_string(),
                    "every",
                ));
            }
            _ => {}
        }
        nodes.push(ui::select(
            "Rounds at most",
            ROUNDS.map(|n| {
                (n.to_string(), if n == 0 { "No limit".to_owned() } else { n.to_string() })
            }),
            config.max_rounds.to_string(),
            "max-rounds",
        ));
        nodes.push(ui::switch(
            "Review the agent's answers to findings",
            config.review_responses,
            "review-responses",
        ));

        if !state.history.is_empty() {
            let rows = state.history.iter().rev().map(|record| {
                let mut outcome = count(record.comments, "comment");
                if record.general > 0 {
                    outcome.push_str(&format!(", {}", count(record.general, "remark")));
                }
                if record.sent {
                    outcome.push_str(", sent");
                }
                if record.stale {
                    outcome.push_str(", stale");
                }
                if !record.problems.is_empty() {
                    outcome.push_str(&format!(" ({})", record.problems.join("; ")));
                }
                vec![record.number.to_string(), record.reviewers.join(", "), outcome]
            });
            nodes.push(ui::table(["Round", "Reviewers", "Outcome"], rows));
        }
        nodes.push(ui::button("Count rounds again", "reset"));
        ui::stack(nodes)
    }

    /// The findings of every round, grouped, with where each stands and what can
    /// be done about it.
    fn findings_section(&mut self, thread: &str, config: &ThreadConfig, nodes: &mut Vec<Node>) {
        let findings = findings_of(thread);
        if findings.is_empty() {
            return;
        }
        let picked = self.picked.entry(thread.to_owned()).or_default();
        picked.retain(|id| findings.iter().any(|f| &f.0.id == id && f.1.pending()));
        let open = findings.iter().filter(|f| f.1 == Status::Open).count();
        nodes.push(ui::divider());
        nodes.push(ui::heading("Findings"));
        let counts: Vec<String> = [Status::Open, Status::Sent, Status::Fixed, Status::Dismissed]
            .iter()
            .map(|status| (status, findings.iter().filter(|f| f.1 == *status).count()))
            .filter(|(_, n)| *n > 0)
            .map(|(status, n)| format!("{n} {}", status.as_str()))
            .collect();
        nodes.push(ui::text(counts.join(" · ")).tone(Tone::Muted));
        let mut actions = Vec::new();
        if open > 0 {
            actions.push(ui::button(format!("Send all open ({open})"), "send-open").primary());
        }
        if !picked.is_empty() {
            actions.push(ui::button(format!("Send selected ({})", picked.len()), "send-picked"));
            actions
                .push(ui::button(format!("Dismiss selected ({})", picked.len()), "dismiss-picked"));
        }
        if !actions.is_empty() {
            nodes.push(ui::row(actions));
        }
        nodes.push(ui::select(
            "Group by",
            [(Grouping::Reviewer.as_str(), "Reviewer"), (Grouping::Severity.as_str(), "Severity")],
            config.grouping.as_str(),
            "grouping",
        ));
        let picked = picked.clone();
        for (title, members) in ledger::grouped(&findings, config.grouping) {
            nodes.push(ui::text(title).tone(Tone::Muted));
            for (record, status, note) in members {
                nodes.push(finding_node(thread, record, *status, note.as_deref(), &picked));
            }
        }
    }

    /// A round is being started: its changes are being read.
    fn starting(&self, thread: &str) -> bool {
        self.diffs
            .values()
            .any(|why| matches!(why, crate::DiffFor::Start { thread: t, .. } if t == thread))
            || self.changes.values().any(|pending| pending.thread == thread)
    }

    fn act(&mut self, thread: &str, action: &str, value: &str) -> Result<Option<String>, String> {
        let mut config: ThreadConfig = self.config_of(thread);
        match action {
            "review" => {
                return Ok(Some(match self.request(thread, None) {
                    Ok(()) => "A review round starts once the diff is read.".to_owned(),
                    Err(error) => format!("No review: {error}."),
                }));
            }
            "reset" => {
                crate::reset(thread);
                return Ok(None);
            }
            "send" => return send_findings(thread, &[value.to_owned()]).map(Some),
            "send-open" => return send_findings(thread, &[]).map(Some),
            "send-picked" | "dismiss-picked" => {
                let ids: Vec<String> =
                    self.picked.remove(thread).unwrap_or_default().into_iter().collect();
                return if action == "send-picked" {
                    send_findings(thread, &ids)
                } else {
                    dismiss_findings(thread, &ids, false)
                }
                .map(Some);
            }
            "dismiss" => return dismiss_findings(thread, &[value.to_owned()], false).map(Some),
            "restore" => return dismiss_findings(thread, &[value.to_owned()], true).map(Some),
            "grouping" => {
                config.grouping =
                    Grouping::parse(value).ok_or_else(|| format!("cannot group by {value:?}"))?;
            }
            _ if action.starts_with("pick:") => {
                let id = &action["pick:".len()..];
                let picked = self.picked.entry(thread.to_owned()).or_default();
                if value == "true" {
                    picked.insert(id.to_owned());
                } else {
                    picked.remove(id);
                }
                return Ok(None);
            }
            "models-refresh" => {
                if let Some(provider) = &config.provider {
                    self.models_of(provider, thread, true);
                }
                return Ok(None);
            }
            // A model and an effort belong to an agent: another agent starts afresh.
            "provider" => {
                let before = config.provider.clone();
                apply(&mut config, action, value)?;
                if config.provider != before {
                    config.model = None;
                    config.effort = None;
                }
            }
            // An effort the new model does not take (or none at all) goes back to the profile's.
            "model" => {
                apply(&mut config, action, value)?;
                if let (Some(provider), Some(effort)) = (&config.provider, &config.effort)
                    && let Some(Models::Ready(models)) = self.models.get(provider)
                    && let Some(model) = chosen_model(models, config.model.as_deref())
                    && !model.efforts.contains(effort)
                {
                    config.effort = None;
                }
            }
            "delay-ms" => {
                config.delay_ms = value.parse().map_err(|_| format!("{value:?} is not a delay"))?;
            }
            _ => match action.strip_prefix("profile:") {
                Some(name) => {
                    config.profiles.retain(|chosen| chosen != name);
                    if value == "true" {
                        config.profiles.push(name.to_owned());
                    }
                }
                None => apply(&mut config, action, value)?,
            },
        }
        self.set_config(thread, &config);
        Ok(None)
    }
}

impl Ui for Reviewers {
    fn views(&self) -> Vec<UiViewInfo> {
        vec![UiViewInfo {
            id: VIEW.into(),
            title: "Reviewers".into(),
            placement: UiPlacement::ThreadPanel,
            scope: UiScope::AllThreads,
            web: None,
        }]
    }

    fn render(&mut self, request: UiRender) -> Result<String, String> {
        let thread = request.thread.ok_or("the reviewers view belongs to a thread")?;
        Ok(self.panel(&thread).to_json())
    }

    fn action(&mut self, action: UiAction) -> Result<String, String> {
        let thread = action.thread.ok_or("the reviewers view belongs to a thread")?;
        let value = action.value.unwrap_or_default();
        let note = match self.act(&thread, &action.action, value.trim()) {
            Ok(note) => note.map(|note| (note, Tone::Muted)),
            Err(error) => Some((error, Tone::Danger)),
        };
        let panel = self.panel(&thread);
        Ok(match note {
            Some((note, tone)) => ui::stack([ui::text(note).tone(tone), panel]).to_json(),
            None => panel.to_json(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, default: bool, efforts: &[&str]) -> ModelInfo {
        ModelInfo {
            id: id.into(),
            name: id.to_uppercase(),
            description: None,
            is_default: default,
            efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
            default_effort: None,
        }
    }

    #[test]
    fn efforts_come_from_the_model_chosen_or_the_default_one() {
        let models = [model("a", false, &["low"]), model("b", true, &["low", "high"])];
        assert_eq!(chosen_model(&models, Some("a")).unwrap().id, "a");
        assert_eq!(chosen_model(&models, None).unwrap().id, "b", "the agent's default");
        assert!(chosen_model(&models, Some("gone")).is_none());
        assert_eq!(chosen_model(&[model("x", false, &[])], None).unwrap().id, "x");
    }

    #[test]
    fn options_start_with_the_profile_and_keep_the_current_value() {
        let listed = || [("a".to_owned(), "A".to_owned())];
        let values = |options: Vec<(String, String)>| -> Vec<String> {
            options.into_iter().map(|(value, _)| value).collect()
        };
        assert_eq!(values(options(listed(), None)), ["", "a"]);
        assert_eq!(values(options(listed(), Some("a"))), ["", "a"]);
        assert_eq!(values(options(listed(), Some("custom"))), ["", "a", "custom"]);
    }
}
