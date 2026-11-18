//! The usage limits of the ChatGPT plan Codex is signed in with.
//!
//! `account/rateLimits/read` answers with them at any time, without a turn:
//! `rateLimitsByLimitId` has a snapshot per metered bucket (`codex`, and buckets of
//! single models), `rateLimits` the main one alone. A snapshot has up to two
//! windows, `primary` and `secondary`, each with `usedPercent`, `windowDurationMins`
//! and `resetsAt` in Unix seconds; which is the 5-hour one and which the weekly one
//! depends on the plan, so a window is named by its length. During a session
//! `account/rateLimits/updated` brings a snapshot after each turn; it is sparse: a
//! window or a plan it does not carry is unknown, not gone.

use qcc_plugin_sdk::types::{LimitStatus, LimitWindow, Outcome, RateLimits};
use serde_json::Value;

use crate::probe::{self, ListProbe};

/// The bucket every Codex request counts against.
const MAIN_BUCKET: &str = "codex";

/// The request id a session reads its limits with: a string, apart from the
/// numbered requests of the session, so asking shifts none of them.
pub const READ_ID: &str = "qcc-rate-limits";

pub const READ: &str = "account/rateLimits/read";

/// The handshake and the question, one line each.
pub fn requests() -> Vec<String> {
    probe::requests(READ)
}

pub fn probe() -> ListProbe {
    ListProbe { what: "usage limits", parse: |result| Outcome::RateLimits(from_read(result)) }
}

/// How a window of so many minutes is called.
fn label(minutes: Option<u64>) -> String {
    match minutes {
        Some(10_080) => "Weekly".to_owned(),
        Some(1_440) => "Daily".to_owned(),
        Some(minutes) if minutes % 1_440 == 0 => format!("{}-day", minutes / 1_440),
        Some(minutes) if minutes % 60 == 0 => format!("{}-hour", minutes / 60),
        Some(minutes) => format!("{minutes}-minute"),
        None => "Usage".to_owned(),
    }
}

/// The windows of one bucket's snapshot.
fn windows(snapshot: &Value, out: &mut Vec<LimitWindow>) {
    let bucket = snapshot["limitId"].as_str().unwrap_or(MAIN_BUCKET);
    let scope = (bucket != MAIN_BUCKET)
        .then(|| snapshot["limitName"].as_str().unwrap_or(bucket).to_owned());
    for slot in ["primary", "secondary"] {
        let window = &snapshot[slot];
        let Some(used) = window["usedPercent"].as_u64() else { continue };
        let minutes = window["windowDurationMins"].as_u64();
        let used = u32::try_from(used).unwrap_or(u32::MAX);
        out.push(LimitWindow {
            id: format!("{bucket}:{slot}"),
            label: label(minutes),
            scope: scope.clone(),
            used_percent: used,
            resets_at: window["resetsAt"].as_u64().map(|seconds| seconds * 1000),
            window_minutes: minutes,
            // The snapshot has no status per window; a spent one is plain to see.
            status: (used >= 100).then_some(LimitStatus::Exceeded),
        });
    }
}

fn plan(snapshot: &Value) -> Option<String> {
    snapshot["planType"].as_str().filter(|plan| *plan != "unknown").map(str::to_owned)
}

/// The answer to `account/rateLimits/read`: every bucket, the main one first.
pub fn from_read(result: &Value) -> RateLimits {
    let mut out = Vec::new();
    let main = &result["rateLimits"];
    match result["rateLimitsByLimitId"].as_object() {
        Some(buckets) if !buckets.is_empty() => {
            if let Some(main) = buckets.get(MAIN_BUCKET) {
                windows(main, &mut out);
            }
            for (id, bucket) in buckets {
                if id != MAIN_BUCKET {
                    windows(bucket, &mut out);
                }
            }
        }
        _ => windows(main, &mut out),
    }
    RateLimits { plan: plan(main), windows: out, complete: true }
}

/// The snapshot of `account/rateLimits/updated`: the windows it carries.
pub fn from_update(params: &Value) -> Option<RateLimits> {
    let snapshot = &params["rateLimits"];
    let mut out = Vec::new();
    windows(snapshot, &mut out);
    let plan = plan(snapshot);
    (!out.is_empty() || plan.is_some()).then_some(RateLimits {
        plan,
        windows: out,
        complete: false,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_read_lists_every_bucket_named_by_window_length() {
        let result = json!({
            "rateLimits": {"limitId": "codex", "planType": "plus",
                           "primary": {"usedPercent": 12, "windowDurationMins": 300,
                                       "resetsAt": 1795000000}},
            "rateLimitsByLimitId": {
                "codex_spark": {"limitId": "codex_spark", "limitName": "Spark",
                                "primary": {"usedPercent": 100, "windowDurationMins": 10080},
                                "rateLimitReachedType": "rate_limit_reached"},
                "codex": {"limitId": "codex", "planType": "plus",
                          "primary": {"usedPercent": 12, "windowDurationMins": 300,
                                      "resetsAt": 1795000000},
                          "secondary": {"usedPercent": 30, "windowDurationMins": 10080}}}});
        let limits = from_read(&result);
        assert!(limits.complete);
        assert_eq!(limits.plan.as_deref(), Some("plus"));
        let rows: Vec<_> = limits
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.label.as_str(), w.scope.as_deref(), w.used_percent))
            .collect();
        assert_eq!(
            rows,
            [
                ("codex:primary", "5-hour", None, 12),
                ("codex:secondary", "Weekly", None, 30),
                ("codex_spark:primary", "Weekly", Some("Spark"), 100),
            ]
        );
        assert_eq!(limits.windows[0].resets_at, Some(1_795_000_000_000));
        assert_eq!(limits.windows[2].status, Some(LimitStatus::Exceeded));
    }

    #[test]
    fn an_update_carries_only_what_it_knows() {
        let params = json!({"rateLimits": {"limitId": "codex", "primary": null,
            "secondary": {"usedPercent": 31, "windowDurationMins": 10080}}});
        let limits = from_update(&params).unwrap();
        assert!(!limits.complete);
        assert_eq!(limits.plan, None);
        assert_eq!(limits.windows.len(), 1);
        assert_eq!(limits.windows[0].id, "codex:secondary");
        assert_eq!(from_update(&json!({"rateLimits": {}})), None);
    }
}
