//! The usage limits of the Claude subscription the CLI is signed in with.
//!
//! Two sources say how much of them is spent:
//!
//! * a `rate_limit_event` after each API response whose limits moved: its
//!   `rate_limit_info` carries the limiting window (`rateLimitType`, `utilization`
//!   as a fraction, `resetsAt` in Unix seconds) and, in `unifiedWindows`, every
//!   window the response's headers told of. Only those windows are known from it.
//! * the `get_usage` control request (the data `/usage` shows), answered from the
//!   claude.ai usage endpoint without a turn: `subscription_type` and
//!   `rate_limits` with `five_hour`, `seven_day`, the per-model weekly windows
//!   (`seven_day_opus`, `seven_day_sonnet`, `model_scoped`, or the scoped rows of
//!   `limits`), each with `utilization` in percent and `resets_at` in ISO 8601.
//!   `rate_limits_available` is false for an API key or a cloud provider, which
//!   have no such limits.

use qcc_plugin_sdk::json::str_field;
use qcc_plugin_sdk::types::{LimitStatus, LimitWindow, Outcome, RateLimits};
use serde_json::{Value, json};

use crate::probe::ControlProbe;

const PROBE_ID: &str = "qcc-limits";

const FIVE_HOURS: u64 = 5 * 60;
const WEEK: u64 = 7 * 24 * 60;

/// The `get_usage` request. Without the scan of local transcripts that `/usage`
/// also shows, which only slows the answer down.
pub fn request(request_id: &str) -> String {
    json!({"type": "control_request", "request_id": request_id,
           "request": {"subtype": "get_usage", "skip_behaviors": true}})
    .to_string()
}

pub fn probe_request() -> String {
    request(PROBE_ID)
}

pub fn probe() -> ControlProbe {
    ControlProbe {
        request_id: PROBE_ID,
        what: "usage limits",
        parse: |response| Outcome::RateLimits(from_usage(response)),
    }
}

/// What a window of the dialect is called, how long it is and whom it is for.
fn known(kind: &str) -> Option<(&'static str, u64, Option<&'static str>)> {
    Some(match kind {
        "five_hour" => ("5-hour", FIVE_HOURS, None),
        "seven_day" => ("Weekly", WEEK, None),
        "seven_day_opus" => ("Weekly", WEEK, Some("Opus")),
        "seven_day_sonnet" => ("Weekly", WEEK, Some("Sonnet")),
        "seven_day_oauth_apps" => ("Weekly", WEEK, Some("Apps")),
        _ => return None,
    })
}

fn status_of(value: &str) -> Option<LimitStatus> {
    match value {
        "allowed" => Some(LimitStatus::Allowed),
        "allowed_warning" => Some(LimitStatus::Warning),
        "rejected" => Some(LimitStatus::Exceeded),
        _ => None,
    }
}

fn window(
    id: String,
    kind: &str,
    scope: Option<String>,
    used: f64,
    resets_at: Option<u64>,
    status: Option<LimitStatus>,
) -> LimitWindow {
    let (label, minutes, known_scope) = known(kind).unwrap_or(("Weekly", WEEK, None));
    LimitWindow {
        id,
        label: label.to_owned(),
        scope: scope.or_else(|| known_scope.map(str::to_owned)),
        used_percent: used.max(0.0).round() as u32,
        resets_at,
        window_minutes: Some(minutes),
        status,
    }
}

/// A `rate_limit_event`'s `rate_limit_info`: the windows it tells of, leaving the
/// others as they were. Nothing when it tells only whether requests are allowed.
pub fn from_event(info: &Value) -> Option<RateLimits> {
    let mut windows = Vec::new();
    if let Some(unified) = info["unifiedWindows"].as_object() {
        for (kind, entry) in unified {
            // The overage-included window is a bucket of extra usage, not a limit
            // of the plan.
            if known(kind).is_none() {
                continue;
            }
            let Some(utilization) = entry["utilization"].as_f64() else { continue };
            windows.push(window(
                kind.clone(),
                kind,
                None,
                utilization * 100.0,
                entry["resetsAt"].as_u64().map(|seconds| seconds * 1000),
                status_of(str_field(entry, "status")),
            ));
        }
    }
    // An older agent tells of the limiting window only.
    let kind = str_field(info, "rateLimitType");
    if windows.is_empty() && known(kind).is_some() {
        let status = status_of(str_field(info, "status"));
        let utilization = match info["utilization"].as_f64() {
            Some(utilization) => utilization,
            // Refused at the limit, which is all it says.
            None if status == Some(LimitStatus::Exceeded) => 1.0,
            None => return None,
        };
        let resets_at = info["resetsAt"].as_u64().map(|seconds| seconds * 1000);
        windows.push(window(kind.to_owned(), kind, None, utilization * 100.0, resets_at, status));
    }
    (!windows.is_empty()).then_some(RateLimits { plan: None, windows, complete: false })
}

/// The answer to `get_usage`: every window the plan has.
pub fn from_usage(response: &Value) -> RateLimits {
    let plan = response["subscription_type"].as_str().map(str::to_owned);
    let limits = &response["rate_limits"];
    let mut windows = Vec::new();
    if response["rate_limits_available"] == false || !limits.is_object() {
        return RateLimits { plan, windows, complete: true };
    }
    for kind in ["five_hour", "seven_day", "seven_day_opus", "seven_day_sonnet"] {
        let entry = &limits[kind];
        let Some(used) = entry["utilization"].as_f64() else { continue };
        let resets_at = entry["resets_at"].as_str().and_then(parse_time);
        windows.push(window(kind.to_owned(), kind, None, used, resets_at, exceeded(used)));
    }
    // Per-model weekly windows: `model_scoped` where the agent sends it, else the
    // scoped rows of the `limits` list the usage screen draws.
    let scoped: Vec<(String, f64, Option<u64>)> = match limits["model_scoped"].as_array() {
        Some(entries) => entries
            .iter()
            .filter_map(|entry| {
                let name = entry["display_name"].as_str()?.to_owned();
                let used = entry["utilization"].as_f64()?;
                Some((name, used, entry["resets_at"].as_str().and_then(parse_time)))
            })
            .collect(),
        None => limits["limits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|entry| entry["kind"] == "weekly_scoped")
            .filter_map(|entry| {
                let name = entry["scope"]["model"]["display_name"].as_str()?.to_owned();
                let used = entry["percent"].as_f64()?;
                Some((name, used, entry["resets_at"].as_str().and_then(parse_time)))
            })
            .collect(),
    };
    for (name, used, resets_at) in scoped {
        let id = format!("seven_day:{}", name.to_lowercase());
        if windows.iter().any(|known: &LimitWindow| known.id == id) {
            continue;
        }
        windows.push(window(id, "seven_day", Some(name), used, resets_at, exceeded(used)));
    }
    RateLimits { plan, windows, complete: true }
}

/// The usage answer has no status of its own; a spent window is plain to see.
fn exceeded(used: f64) -> Option<LimitStatus> {
    (used >= 100.0).then_some(LimitStatus::Exceeded)
}

/// An ISO 8601 time (`2026-11-18T21:20:00.620751+00:00`) in milliseconds since the
/// Unix epoch.
fn parse_time(text: &str) -> Option<u64> {
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let mut rest = &text[19..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.find(|c: char| !c.is_ascii_digit()).unwrap_or(fraction.len());
        let padded = format!("{:0<3}", &fraction[..digits.min(3)]);
        millis = padded.parse::<i64>().ok()?;
        rest = &fraction[digits..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let hours = rest.get(1..3)?.parse::<i64>().ok()?;
            let minutes = rest.get(4..6).and_then(|m| m.parse::<i64>().ok()).unwrap_or(0);
            sign * (hours * 60 + minutes) * 60
        }
    };
    // Days from the civil date (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60 + second - offset;
    u64::try_from(seconds * 1000 + millis).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_with_their_offset() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-11-18T21:20:00.620751+00:00"), Some(1_795_036_800_620));
        assert_eq!(parse_time("2026-11-18T23:20:00+02:00"), Some(1_795_036_800_000));
        assert_eq!(parse_time("soon"), None);
    }

    #[test]
    fn an_event_reports_the_windows_it_tells_of() {
        let info = json!({"status": "allowed", "resetsAt": 1795036800, "rateLimitType": "five_hour",
            "unifiedWindows": {
                "five_hour": {"utilization": 0.42, "resetsAt": 1795036800, "status": "allowed"},
                "seven_day": {"utilization": 0.43, "resetsAt": 1795053600,
                              "status": "allowed_warning"},
                "seven_day_overage_included": {"utilization": 0.1, "resetsAt": 1795053600}}});
        let limits = from_event(&info).unwrap();
        assert!(!limits.complete);
        let windows: Vec<_> = limits
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.used_percent, w.resets_at, w.status))
            .collect();
        assert_eq!(
            windows,
            [
                ("five_hour", 42, Some(1_795_036_800_000), Some(LimitStatus::Allowed)),
                ("seven_day", 43, Some(1_795_053_600_000), Some(LimitStatus::Warning)),
            ]
        );
        assert_eq!(limits.windows[1].window_minutes, Some(WEEK));
    }

    #[test]
    fn an_older_event_reports_the_limiting_window_and_a_bare_one_nothing() {
        let info = json!({"status": "rejected", "rateLimitType": "seven_day_opus",
                          "resetsAt": 1795053600});
        let limits = from_event(&info).unwrap();
        assert_eq!(limits.windows[0].used_percent, 100);
        assert_eq!(limits.windows[0].scope.as_deref(), Some("Opus"));
        assert_eq!(limits.windows[0].status, Some(LimitStatus::Exceeded));
        assert_eq!(from_event(&json!({"status": "allowed"})), None);
    }

    #[test]
    fn the_usage_answer_lists_every_window_of_the_plan() {
        let response = json!({"subscription_type": "max", "rate_limits_available": true,
            "rate_limits": {
                "five_hour": {"utilization": 42, "resets_at": "2026-11-18T21:20:00.620751+00:00"},
                "seven_day": {"utilization": 44.4, "resets_at": "2026-11-19T02:00:00+00:00"},
                "seven_day_opus": null,
                "limits": [
                    {"kind": "session", "percent": 42},
                    {"kind": "weekly_scoped", "percent": 26,
                     "resets_at": "2026-11-19T01:59:59.620970+00:00",
                     "scope": {"model": {"id": null, "display_name": "Fable"}}}]}});
        let limits = from_usage(&response);
        assert!(limits.complete);
        assert_eq!(limits.plan.as_deref(), Some("max"));
        let windows: Vec<_> = limits
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.label.as_str(), w.scope.as_deref(), w.used_percent))
            .collect();
        assert_eq!(
            windows,
            [
                ("five_hour", "5-hour", None, 42),
                ("seven_day", "Weekly", None, 44),
                ("seven_day:fable", "Weekly", Some("Fable"), 26),
            ]
        );
        assert_eq!(limits.windows[2].resets_at, Some(1_795_053_599_620));
    }

    #[test]
    fn an_account_without_plan_limits_has_no_windows() {
        let response = json!({"subscription_type": null, "rate_limits_available": false,
                              "rate_limits": null});
        assert_eq!(
            from_usage(&response),
            RateLimits { plan: None, windows: vec![], complete: true }
        );
    }
}
