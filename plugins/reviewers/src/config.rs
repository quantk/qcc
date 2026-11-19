//! What the user chose for a thread, and what the plugin remembers about its reviews.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::findings::{Finding, Severity};
use crate::profile::Delivery;

/// When a review starts on its own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// The agent finished: a turn completed, and no new one started for a while.
    #[default]
    Finished,
    /// After every completed turn.
    EveryTurn,
    /// After every so many completed turns.
    EveryTurns,
    /// Only when someone asks: the user, or the agent with `qcc reviewers request`.
    Manual,
    /// When the agent commits. Not wired up yet: works as `manual`.
    Commit,
    /// Before the work is delivered. Not wired up yet: works as `manual`.
    Deliver,
}

impl Trigger {
    /// The triggers that work now, as the UI and the CLI offer them.
    pub const OFFERED: [Self; 4] =
        [Self::Finished, Self::EveryTurn, Self::EveryTurns, Self::Manual];

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.trim() {
            "finished" | "done" => Self::Finished,
            "every-turn" | "turn" => Self::EveryTurn,
            "every-turns" | "every-n" => Self::EveryTurns,
            "manual" => Self::Manual,
            "commit" => Self::Commit,
            "deliver" => Self::Deliver,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::EveryTurn => "every-turn",
            Self::EveryTurns => "every-turns",
            Self::Manual => "manual",
            Self::Commit => "commit",
            Self::Deliver => "deliver",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Finished => "When the agent is done",
            Self::EveryTurn => "After every turn",
            Self::EveryTurns => "Every few turns",
            Self::Manual => "Only when asked",
            Self::Commit => "On every commit (not yet)",
            Self::Deliver => "Before delivering (not yet)",
        }
    }
}

/// The reviews of one thread, as the user set them up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThreadConfig {
    /// Reviews start on their own, by `trigger`. A review asked for by hand runs
    /// either way.
    pub enabled: bool,
    /// The profiles that review, by name.
    pub profiles: Vec<String>,
    /// Override every profile's agent, model and effort.
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub delivery: Delivery,
    pub trigger: Trigger,
    /// For `every-turns`.
    pub every: u32,
    /// For `finished`: how long the agent must stay idle.
    pub delay_ms: u64,
    /// Rounds at most; 0 for no limit.
    pub max_rounds: u32,
    /// The turn in which the agent answers the findings of a review may start the
    /// next one.
    pub review_responses: bool,
}

impl Default for ThreadConfig {
    fn default() -> Self {
        Defaults::default().thread_config()
    }
}

/// What a thread starts with: the plugin's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    pub profiles: Vec<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub delivery: Delivery,
    pub trigger: Trigger,
    pub delay_ms: u64,
    pub max_rounds: u32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            profiles: vec!["general".to_owned()],
            provider: None,
            model: None,
            effort: None,
            delivery: Delivery::Collect,
            trigger: Trigger::Finished,
            delay_ms: 30_000,
            max_rounds: 3,
        }
    }
}

impl Defaults {
    /// From the plugin's settings by id; what is missing or unreadable keeps its default.
    pub fn from_settings(values: &[(String, String)]) -> Self {
        let mut defaults = Self::default();
        let text = |value: &str| Some(value.trim().to_owned()).filter(|v| !v.is_empty());
        for (key, value) in values {
            match key.as_str() {
                "profiles" => {
                    defaults.profiles = value
                        .split(',')
                        .map(|name| name.trim().to_owned())
                        .filter(|name| !name.is_empty())
                        .collect();
                }
                "provider" => defaults.provider = text(value),
                "model" => defaults.model = text(value),
                "effort" => defaults.effort = text(value),
                "delivery" => {
                    defaults.delivery = Delivery::parse(value).unwrap_or(defaults.delivery);
                }
                "trigger" => defaults.trigger = Trigger::parse(value).unwrap_or(defaults.trigger),
                "delay" => {
                    if let Some(ms) = parse_delay(value) {
                        defaults.delay_ms = ms;
                    }
                }
                "max_rounds" => {
                    defaults.max_rounds = value.trim().parse().unwrap_or(defaults.max_rounds);
                }
                _ => {}
            }
        }
        defaults
    }

    pub fn thread_config(&self) -> ThreadConfig {
        ThreadConfig {
            enabled: false,
            profiles: self.profiles.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            delivery: self.delivery,
            trigger: self.trigger,
            every: 3,
            delay_ms: self.delay_ms,
            max_rounds: self.max_rounds,
            review_responses: false,
        }
    }
}

/// `30`, `30s`, `1.5s`, `2m` or `500ms`, as milliseconds.
pub fn parse_delay(text: &str) -> Option<u64> {
    let text = text.trim();
    let (number, scale) = if let Some(ms) = text.strip_suffix("ms") {
        (ms, 1.0)
    } else if let Some(m) = text.strip_suffix('m') {
        (m, 60_000.0)
    } else {
        (text.strip_suffix('s').unwrap_or(text), 1000.0)
    };
    let value: f64 = number.trim().parse().ok()?;
    (value.is_finite() && value >= 0.0).then(|| (value * scale).round() as u64)
}

/// A delay for people: `30s`, `2m`, `500ms`.
pub fn format_delay(ms: u64) -> String {
    if ms.is_multiple_of(60_000) && ms > 0 {
        format!("{}m", ms / 60_000)
    } else if ms.is_multiple_of(1000) {
        format!("{}s", ms / 1000)
    } else {
        format!("{ms}ms")
    }
}

/// One reviewer of a round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Critic {
    pub profile: String,
    /// Its thread, a child of the reviewed one.
    pub thread: String,
    pub provider: String,
    pub model: Option<String>,
    pub delivery: Delivery,
    pub severity: Severity,
    pub done: bool,
    pub findings: Vec<Finding>,
    /// What went wrong, when it did not report findings.
    pub problem: Option<String>,
}

/// A review in flight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub number: u32,
    /// Of the diff the reviewers were given.
    pub fingerprint: String,
    pub critics: Vec<Critic>,
}

/// A finished round, for the panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoundRecord {
    pub number: u32,
    pub reviewers: Vec<String>,
    /// Comments left.
    pub comments: u32,
    /// Findings no comment could hold (no file or line).
    pub general: u32,
    /// The changes moved on while it ran.
    pub stale: bool,
    /// The findings were sent to the agent.
    pub sent: bool,
    pub problems: Vec<String>,
}

/// What the plugin remembers about one thread's reviews.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Progress {
    /// Completed turns, for `every-turns`.
    pub turns: u32,
    /// Rounds started, for the limit.
    pub rounds: u32,
    /// The diff the last round reviewed.
    pub fingerprint: Option<String>,
    /// Each file's part of it.
    pub files: BTreeMap<String, String>,
    pub round: Option<Round>,
    /// Findings were sent to the agent: the turn ending this many turn ends from
    /// now answers them. 0 when none is expected.
    pub awaiting_response: u32,
    /// Newest last, a few.
    pub history: Vec<RoundRecord>,
    /// Why the last attempt to start a review did not, for the panel.
    pub note: Option<String>,
}

/// How many finished rounds the panel keeps.
pub const HISTORY: usize = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_fill_in_the_defaults() {
        let values = [
            ("profiles", "general, security"),
            ("provider", "codex"),
            ("model", " "),
            ("delivery", "auto"),
            ("delay", "1.5s"),
            ("max_rounds", "x"),
        ]
        .map(|(k, v)| (k.to_owned(), v.to_owned()));
        let defaults = Defaults::from_settings(&values);
        assert_eq!(defaults.profiles, ["general", "security"]);
        assert_eq!(defaults.provider.as_deref(), Some("codex"));
        assert_eq!(defaults.model, None);
        assert_eq!(defaults.delivery, Delivery::Auto);
        assert_eq!(defaults.delay_ms, 1500);
        assert_eq!(defaults.max_rounds, 3);
        let config = defaults.thread_config();
        assert!(!config.enabled && config.trigger == Trigger::Finished);
    }

    #[test]
    fn delays_read_and_print() {
        assert_eq!(parse_delay("30"), Some(30_000));
        assert_eq!(parse_delay("2m"), Some(120_000));
        assert_eq!(parse_delay("250ms"), Some(250));
        assert_eq!(parse_delay("-1"), None);
        assert_eq!(format_delay(30_000), "30s");
        assert_eq!(format_delay(120_000), "2m");
        assert_eq!(format_delay(250), "250ms");
    }

    #[test]
    fn old_state_reads_with_new_fields_defaulted() {
        let state: Progress = serde_json::from_str("{\"rounds\": 2}").unwrap();
        assert_eq!(state.rounds, 2);
        assert!(state.round.is_none());
        let config: ThreadConfig = serde_json::from_str("{\"enabled\": true}").unwrap();
        assert!(config.enabled);
        assert_eq!(config.delay_ms, 30_000);
    }
}
