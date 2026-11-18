//! How much of a subscription's usage limits an agent's account has spent.
//!
//! Limits belong to the account a provider's agent signs in with, not to a thread:
//! every thread of the provider spends the same allowance. A bridge reports them as
//! its agent tells (a rate-limit event during a turn, or an answer to a question
//! asked without one), and the core keeps the latest per provider.

use serde::{Deserialize, Serialize};

use crate::ids::ProviderId;

/// Where a window stands, as the agent's service says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    /// Within the limit.
    Allowed,
    /// Past a warning threshold, still allowed.
    Warning,
    /// The limit is reached: requests that count against it are refused (or paid for
    /// as extra usage, where the account allows it).
    Exceeded,
}

/// One usage window: a share of an allowance that starts over at a set time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LimitWindow {
    /// Stable within the provider, so a later report of the same window replaces
    /// this one (`five_hour`, `seven_day`, `seven_day:fable`, `codex:primary`).
    pub id: String,
    /// How the agent's service names the window, for people (`5-hour`, `Weekly`).
    pub label: String,
    /// What the window is limited to when not everything counts against it: a model
    /// (`Opus`), a product line. Absent for the account's main windows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// How much of the allowance is spent, in percent. Can go past 100.
    pub used_percent: u32,
    /// When the window starts over, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<u64>,
    /// How long the window is, when the service says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<LimitStatus>,
}

/// What a provider reported about its account's limits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RateLimits {
    /// The subscription the account is on (`max`, `pro`, `plus`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub windows: Vec<LimitWindow>,
    /// Every window the account has is listed: one known before and missing here is
    /// gone. Otherwise the report updates the windows it lists and leaves the others.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub complete: bool,
}

/// The latest known limits of one provider's account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ProviderLimits {
    pub provider: ProviderId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    /// In the order the provider reported them.
    pub windows: Vec<LimitWindow>,
    /// When the provider last reported, in milliseconds since the Unix epoch.
    pub updated_at: u64,
}

impl ProviderLimits {
    pub fn new(provider: ProviderId, report: RateLimits, now: u64) -> Self {
        let mut limits = Self { provider, plan: None, windows: Vec::new(), updated_at: now };
        limits.merge(RateLimits { complete: true, ..report }, now);
        limits
    }

    /// Take a newer report in: a complete one replaces what was known, a partial one
    /// updates the windows it lists. A plan not mentioned is kept.
    pub fn merge(&mut self, report: RateLimits, now: u64) {
        if report.plan.is_some() {
            self.plan = report.plan;
        }
        if report.complete {
            self.windows = report.windows;
        } else {
            for window in report.windows {
                match self.windows.iter_mut().find(|known| known.id == window.id) {
                    Some(known) => *known = window,
                    None => self.windows.push(window),
                }
            }
        }
        self.updated_at = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(id: &str, used: u32) -> LimitWindow {
        LimitWindow {
            id: id.into(),
            label: id.into(),
            scope: None,
            used_percent: used,
            resets_at: None,
            window_minutes: None,
            status: None,
        }
    }

    #[test]
    fn a_partial_report_updates_its_windows_and_keeps_the_rest() {
        let full = RateLimits {
            plan: Some("max".into()),
            windows: vec![window("five_hour", 10), window("seven_day", 20)],
            complete: true,
        };
        let mut limits = ProviderLimits::new("claude-code".into(), full, 1);
        let partial = RateLimits {
            plan: None,
            windows: vec![window("seven_day", 25), window("seven_day:opus", 3)],
            complete: false,
        };
        limits.merge(partial, 2);
        let used: Vec<_> = limits.windows.iter().map(|w| (w.id.as_str(), w.used_percent)).collect();
        assert_eq!(used, [("five_hour", 10), ("seven_day", 25), ("seven_day:opus", 3)]);
        assert_eq!(limits.plan.as_deref(), Some("max"));
        assert_eq!(limits.updated_at, 2);

        let complete =
            RateLimits { plan: None, windows: vec![window("five_hour", 0)], complete: true };
        limits.merge(complete, 3);
        assert_eq!(limits.windows, [window("five_hour", 0)]);
    }

    #[test]
    fn a_first_report_is_taken_whole() {
        let partial =
            RateLimits { plan: None, windows: vec![window("five_hour", 7)], complete: false };
        let limits = ProviderLimits::new("codex".into(), partial, 5);
        assert_eq!(limits.windows, [window("five_hour", 7)]);
    }
}
