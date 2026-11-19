//! The thread's reviewer settings, in the panel beside it.

use qcc_plugin_sdk::prelude::*;
use qcc_plugin_sdk::ui::{self, Node, Tone};

use crate::config::{self, ThreadConfig, Trigger};
use crate::profile::Delivery;
use crate::{Reviewers, apply, profiles_for, state_of};

pub(crate) const VIEW: &str = "reviewers";

const DELAYS: [u64; 6] = [5_000, 15_000, 30_000, 60_000, 120_000, 300_000];
const ROUNDS: [u32; 6] = [1, 2, 3, 5, 10, 0];
const EVERY: [u32; 4] = [2, 3, 5, 10];

impl Reviewers {
    fn panel(&self, thread: &str) -> Node {
        let config = self.config_of(thread);
        let state = state_of(thread);
        let (profiles, problems) = profiles_for(thread);
        let mut nodes = vec![
            ui::heading("Reviewers"),
            ui::switch("Review this thread on its own", config.enabled, "enabled"),
        ];

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

        let mut agents: Vec<(String, String)> = vec![(String::new(), "As the profile says".into())];
        if let Ok(providers) = threads::providers() {
            agents.extend(providers.into_iter().map(|p| (p.id, p.name)));
        }
        nodes.push(ui::select(
            "Agent",
            agents,
            config.provider.clone().unwrap_or_default(),
            "provider",
        ));
        nodes.push(ui::text(format!(
            "Model: {} · Effort: {}",
            config.model.as_deref().unwrap_or("as the profile says"),
            config.effort.as_deref().unwrap_or("as the profile says"),
        )));
        nodes.push(
            ui::input("Model (empty: the profile's)", "Set model", "model")
                .value(config.model.clone().unwrap_or_default()),
        );
        nodes.push(
            ui::input("Effort (empty: the profile's)", "Set effort", "effort")
                .value(config.effort.clone().unwrap_or_default()),
        );

        nodes.push(ui::select(
            "Findings",
            [
                (Delivery::Collect.as_str(), "Leave as comments for me to send"),
                (Delivery::Auto.as_str(), "Also send them to the agent"),
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

        nodes.push(ui::divider());
        let limit = if config.max_rounds == 0 {
            "no limit".to_owned()
        } else {
            config.max_rounds.to_string()
        };
        let mut status = vec![("Rounds".to_owned(), format!("{} of {limit}", state.rounds))];
        if let Some(round) = &state.round {
            let done = round.critics.iter().filter(|c| c.done).count();
            status.push((
                "Running".to_owned(),
                format!("round {}: {done} of {} reviewers done", round.number, round.critics.len()),
            ));
        } else if self.armed(thread) {
            status.push(("Next".to_owned(), "once the agent stays idle".to_owned()));
        }
        nodes.push(ui::key_values(status));
        if let Some(note) = &state.note {
            nodes.push(ui::text(note.clone()).tone(Tone::Muted));
        }
        if !state.history.is_empty() {
            let rows = state.history.iter().rev().map(|record| {
                let mut outcome = format!("{} comments", record.comments);
                if record.general > 0 {
                    outcome.push_str(&format!(", {} general", record.general));
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
        let review = ui::button("Review now", "review").primary();
        nodes.push(ui::row([review, ui::button("Count rounds again", "reset")]));
        ui::stack(nodes)
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
