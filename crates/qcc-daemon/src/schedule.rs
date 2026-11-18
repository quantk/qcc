//! When automations run: cron expressions and plain intervals.
//!
//! A schedule is kept as the text the user wrote and parsed here, so the daemon
//! and every client agree on one grammar:
//!
//! * five cron fields, `minute hour day-of-month month day-of-week`, each a `*`, a
//!   number or name (`jan`, `mon`), a range `a-b`, a step `*/n` or `a-b/n`, or a
//!   comma-separated list of those. When both day fields are restricted, a day
//!   matching either one counts, as in classic cron;
//! * the shorthands `@hourly`, `@daily` (`@midnight`), `@weekly`, `@monthly`,
//!   `@yearly` (`@annually`);
//! * an interval: `every 30m`, `every 1h30m`, `every 2d`, `every 45s` (units `s`,
//!   `m`, `h`, `d`; `@every` works too). An interval counts from the previous run.
//!
//! Cron times are wall-clock times of a time zone, the daemon's local one in use.

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Schedule {
    /// Milliseconds between runs.
    Every(u64),
    Cron(Cron),
}

/// The times a cron expression matches, field by field, as bit sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    /// Sunday is bit 0.
    weekdays: u64,
    /// The day fields were `*`: only the other one restricts the day.
    any_day: bool,
    any_weekday: bool,
}

/// How far ahead a cron expression is searched for its next time. A date that
/// exists only in some years (February 29 on a Monday) can be decades apart.
const SEARCH_DAYS: i32 = 366 * 30;

const MONTHS: [&str; 12] =
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

impl Schedule {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let lower = text.to_ascii_lowercase();
        if let Some(interval) =
            lower.strip_prefix("every ").or_else(|| lower.strip_prefix("@every "))
        {
            return parse_interval(interval.trim()).map(Self::Every);
        }
        let expression = match lower.as_str() {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            "@yearly" | "@annually" => "0 0 1 1 *",
            other if other.starts_with('@') => {
                return Err(format!(
                    "unknown schedule {text:?}: use @hourly, @daily, @weekly, @monthly or \
                     @yearly, `every <interval>` or a cron expression"
                ));
            }
            other => other,
        };
        Cron::parse(expression).map(Self::Cron)
    }

    /// The first scheduled time strictly after `after`, in milliseconds since the
    /// Unix epoch. `None` only for a cron expression that names no real date
    /// (`0 0 31 2 *`).
    pub(crate) fn next_after(&self, after: u64, zone: &TimeZone) -> Option<u64> {
        match self {
            Self::Every(period) => after.checked_add(*period),
            Self::Cron(cron) => cron.next_after(after, zone),
        }
    }
}

/// `30m`, `1h30m`, `2d`, `45s`: one or more numbers, each with its unit.
fn parse_interval(text: &str) -> Result<u64, String> {
    let invalid =
        || format!("cannot read the interval {text:?}: write it like 30m, 1h30m, 2d or 45s");
    let mut total: u64 = 0;
    let mut digits = String::new();
    for ch in text.chars().filter(|ch| !ch.is_whitespace()) {
        if ch.is_ascii_digit() {
            digits.push(ch);
            continue;
        }
        let unit: u64 = match ch {
            's' => 1_000,
            'm' => 60_000,
            'h' => 3_600_000,
            'd' => 86_400_000,
            _ => return Err(invalid()),
        };
        let count: u64 = digits.parse().map_err(|_| invalid())?;
        digits.clear();
        total = count.checked_mul(unit).and_then(|ms| total.checked_add(ms)).ok_or_else(invalid)?;
    }
    if !digits.is_empty() || total == 0 {
        return Err(invalid());
    }
    Ok(total)
}

impl Cron {
    fn parse(expression: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        let [minute, hour, day, month, weekday] = fields[..] else {
            return Err(format!(
                "a cron expression has five fields (minute hour day month weekday), \
                 {expression:?} has {}",
                fields.len()
            ));
        };
        let mut weekdays = parse_field(weekday, "weekday", 0, 7, &WEEKDAYS, 0)?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays | 1) & !(1 << 7);
        }
        Ok(Self {
            minutes: parse_field(minute, "minute", 0, 59, &[], 0)?,
            hours: parse_field(hour, "hour", 0, 23, &[], 0)?,
            days: parse_field(day, "day of the month", 1, 31, &[], 0)?,
            months: parse_field(month, "month", 1, 12, &MONTHS, 1)?,
            weekdays,
            any_day: day == "*",
            any_weekday: weekday == "*",
        })
    }

    fn matches_day(&self, date: Date) -> bool {
        if self.months & (1 << date.month()) == 0 {
            return false;
        }
        let day = self.days & (1 << date.day()) != 0;
        let weekday = self.weekdays & (1 << date.weekday().to_sunday_zero_offset()) != 0;
        match (self.any_day, self.any_weekday) {
            (true, true) => true,
            (false, true) => day,
            (true, false) => weekday,
            (false, false) => day || weekday,
        }
    }

    fn next_after(&self, after: u64, zone: &TimeZone) -> Option<u64> {
        let after = i64::try_from(after).ok()?;
        let start = Timestamp::from_millisecond(after).ok()?.to_zoned(zone.clone());
        let mut date = start.date();
        for _ in 0..SEARCH_DAYS {
            if self.matches_day(date) {
                for hour in bits(self.hours) {
                    for minute in bits(self.minutes) {
                        let civil = date.at(hour as i8, minute as i8, 0, 0);
                        // A time the clock skips when it goes forward runs just
                        // after the jump; one it passes twice runs the first time.
                        let Ok(zoned) = zone.to_ambiguous_zoned(civil).compatible() else {
                            continue;
                        };
                        let at = zoned.timestamp().as_millisecond();
                        if at > after {
                            return u64::try_from(at).ok();
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

/// The set bits of a field, lowest first.
fn bits(set: u64) -> impl Iterator<Item = u32> {
    (0..64).filter(move |bit| set & (1 << bit) != 0)
}

/// One cron field as a bit set. `names` stand for `first_name`, `first_name + 1`, ….
fn parse_field(
    text: &str,
    what: &str,
    min: u32,
    max: u32,
    names: &[&str],
    first_name: u32,
) -> Result<u64, String> {
    let value = |part: &str| -> Result<u32, String> {
        let lower = part.to_ascii_lowercase();
        if let Some(index) = names.iter().position(|name| *name == lower) {
            return Ok(first_name + index as u32);
        }
        let number: u32 =
            part.parse().map_err(|_| format!("{part:?} is not a valid {what} in a cron field"))?;
        if !(min..=max).contains(&number) {
            return Err(format!("{what} {number} is out of range {min}-{max}"));
        }
        Ok(number)
    };
    let mut set = 0u64;
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(|| format!("{item:?}: the step must be a positive number"))?;
                (range, step)
            }
            None => (item, 1),
        };
        let (from, to) = if range == "*" {
            (min, max)
        } else if let Some((from, to)) = range.split_once('-') {
            (value(from)?, value(to)?)
        } else {
            let from = value(range)?;
            // `5/15` means from 5 to the end, every 15.
            (from, if item.contains('/') { max } else { from })
        };
        if from > to {
            return Err(format!("{item:?}: the range runs backwards"));
        }
        for bit in (from..=to).step_by(step as usize) {
            set |= 1 << bit;
        }
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(text: &str) -> u64 {
        let ts: Timestamp = text.parse().expect("a timestamp");
        u64::try_from(ts.as_millisecond()).unwrap()
    }

    fn next(schedule: &str, after: &str) -> String {
        let schedule = Schedule::parse(schedule).expect("a valid schedule");
        let at = schedule.next_after(utc(after), &TimeZone::UTC).expect("a next time");
        Timestamp::from_millisecond(at as i64).unwrap().to_string()
    }

    #[test]
    fn intervals_are_read_in_any_mix_of_units() {
        assert_eq!(Schedule::parse("every 30m"), Ok(Schedule::Every(30 * 60_000)));
        assert_eq!(Schedule::parse("Every 1h30m"), Ok(Schedule::Every(90 * 60_000)));
        assert_eq!(Schedule::parse("@every 2d"), Ok(Schedule::Every(2 * 86_400_000)));
        assert_eq!(Schedule::parse("every 1h 15s"), Ok(Schedule::Every(3_615_000)));
        for wrong in ["every", "every 0m", "every 5", "every 5w", "every m", "every -1h"] {
            assert!(Schedule::parse(wrong).is_err(), "{wrong:?} was accepted");
        }
    }

    #[test]
    fn an_interval_counts_from_the_previous_run() {
        assert_eq!(next("every 1h", "2026-03-01T10:17:00Z"), "2026-03-01T11:17:00Z");
    }

    #[test]
    fn cron_fields_take_lists_ranges_steps_and_names() {
        assert_eq!(next("*/15 * * * *", "2026-03-01T10:17:00Z"), "2026-03-01T10:30:00Z");
        assert_eq!(next("0 9 * * mon-fri", "2026-03-06T09:00:00Z"), "2026-03-09T09:00:00Z");
        assert_eq!(next("5,35 8-9 * * *", "2026-03-01T09:35:00Z"), "2026-03-02T08:05:00Z");
        assert_eq!(next("10/20 * * * *", "2026-03-01T10:31:00Z"), "2026-03-01T10:50:00Z");
        assert_eq!(next("0 0 1 jan,jul *", "2026-03-01T00:00:00Z"), "2026-07-01T00:00:00Z");
        // 7 is Sunday as well as 0.
        assert_eq!(next("0 12 * * 7", "2026-03-02T00:00:00Z"), "2026-03-08T12:00:00Z");
    }

    #[test]
    fn shorthands_mean_what_cron_means() {
        assert_eq!(next("@hourly", "2026-03-01T10:17:00Z"), "2026-03-01T11:00:00Z");
        assert_eq!(next("@daily", "2026-03-01T10:17:00Z"), "2026-03-02T00:00:00Z");
        assert_eq!(next("@weekly", "2026-03-02T10:00:00Z"), "2026-03-08T00:00:00Z");
        assert_eq!(next("@monthly", "2026-03-01T00:00:00Z"), "2026-04-01T00:00:00Z");
        assert_eq!(next("@yearly", "2026-03-01T00:00:00Z"), "2027-01-01T00:00:00Z");
    }

    #[test]
    fn restricting_both_day_fields_matches_either() {
        // The 13th, or any Friday: Friday 2026-03-06 comes first.
        assert_eq!(next("0 0 13 * fri", "2026-03-01T00:00:00Z"), "2026-03-06T00:00:00Z");
        // Only the 13th when the weekday is left open.
        assert_eq!(next("0 0 13 * *", "2026-03-01T00:00:00Z"), "2026-03-13T00:00:00Z");
    }

    #[test]
    fn rare_and_impossible_dates() {
        assert_eq!(next("0 0 29 2 *", "2026-03-01T00:00:00Z"), "2028-02-29T00:00:00Z");
        let never = Schedule::parse("0 0 31 2 *").unwrap();
        assert_eq!(never.next_after(utc("2026-03-01T00:00:00Z"), &TimeZone::UTC), None);
    }

    #[test]
    fn mistakes_are_explained() {
        for wrong in [
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "x * * * *",
            "@sometimes",
        ] {
            assert!(Schedule::parse(wrong).is_err(), "{wrong:?} was accepted");
        }
    }

    #[test]
    fn cron_follows_the_local_clock_across_daylight_saving() {
        let berlin = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        let daily = Schedule::parse("0 9 * * *").unwrap();
        // 09:00 is 08:00 UTC in winter and 07:00 UTC in summer.
        let winter = daily.next_after(utc("2026-03-27T12:00:00Z"), &berlin).unwrap();
        assert_eq!(winter, utc("2026-03-28T08:00:00Z"));
        let summer = daily.next_after(winter, &berlin).unwrap();
        assert_eq!(summer, utc("2026-03-29T07:00:00Z"));
        // 02:30 does not exist on the day the clock jumps from 02:00 to 03:00; the
        // run comes right after the jump.
        let skipped = Schedule::parse("30 2 * * *").unwrap();
        let at = skipped.next_after(utc("2026-03-28T12:00:00Z"), &berlin).unwrap();
        assert_eq!(at, utc("2026-03-29T01:30:00Z"));
    }
}
