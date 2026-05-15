//! Recurrence rules for the bulk-create coaching-session flow.
//!
//! Pure logic: given a start datetime and a [`Recurrence`] rule, [`expand_recurrence`]
//! produces the list of occurrence datetimes that the recurring endpoint will then
//! materialize as `coaching_sessions` rows. [`validate_recurrence`] performs the
//! structural checks (interval/terminator/weekday) that are independent of expansion.
//!
//! Two upper bounds keep the expansion finite:
//! - [`MAX_RECURRING_OCCURRENCES`] caps the total number of generated events
//!   (covers daily-for-a-year worst case).
//! - [`MAX_RECURRING_SPAN_DAYS`] caps the calendar span from the first to the last
//!   event (covers leap-year monthly).
//!
//! Times are operated on as [`NaiveDateTime`]; recurrence expansion in naive time
//! can drift up to an hour across DST transitions, which matches how
//! `coaching_sessions.date` is stored today.

use chrono::{Datelike, Duration, Months, NaiveDateTime, Weekday};
use serde::{Deserialize, Serialize};

/// Maximum number of occurrences a single recurrence rule may produce.
const MAX_RECURRING_OCCURRENCES: usize = 365;

/// Maximum calendar span (in days) from the first occurrence to the last.
const MAX_RECURRING_SPAN_DAYS: i64 = 366;

/// How often the rule repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Daily,
    Weekly,
    Biweekly,
    Monthly,
}

/// A recurrence rule. Exactly one of `count` or `until` must be specified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    pub frequency: Frequency,
    /// Step multiplier. `weekly` with `interval: 2` is the same as `biweekly`.
    /// Defaults to 1 if omitted in JSON.
    #[serde(default = "default_interval")]
    pub interval: u32,
    /// Only meaningful for `weekly`/`biweekly`. If present, the rule emits one
    /// event per listed weekday within each active week.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_weekdays: Option<Vec<Weekday>>,
    /// Stop after this many occurrences. Mutually exclusive with `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Stop after this datetime. Mutually exclusive with `count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDateTime>,
}

fn default_interval() -> u32 {
    1
}

/// Why a recurrence rule was rejected. Mapped to HTTP 422 at the web boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum RecurrenceError {
    InvalidInterval,
    AmbiguousTerminator,
    ByWeekdaysOnNonWeekly,
    EmptyByWeekdays,
    StartWeekdayMismatch,
    TooManyOccurrences { count: usize },
    SpanTooLong { days: i64 },
    NoOccurrencesGenerated,
}

impl RecurrenceError {
    /// Human-readable message suitable for the `ValidationError { message }`
    /// payload returned to API clients.
    pub fn message(&self) -> String {
        match self {
            Self::InvalidInterval => "`interval` must be at least 1".into(),
            Self::AmbiguousTerminator => {
                "exactly one of `count` or `until` must be provided".into()
            }
            Self::ByWeekdaysOnNonWeekly => {
                "`by_weekdays` is only valid for weekly or biweekly frequencies".into()
            }
            Self::EmptyByWeekdays => "`by_weekdays` cannot be empty when provided".into(),
            Self::StartWeekdayMismatch => {
                "`start_at`'s weekday must be included in `by_weekdays`".into()
            }
            Self::TooManyOccurrences { count } => format!(
                "recurrence would produce {count} occurrences (max {MAX_RECURRING_OCCURRENCES})"
            ),
            Self::SpanTooLong { days } => format!(
                "recurrence span of {days} days exceeds the maximum of {MAX_RECURRING_SPAN_DAYS}"
            ),
            Self::NoOccurrencesGenerated => {
                "recurrence rule produced no occurrences (check `until` vs. `start_at`)".into()
            }
        }
    }
}

/// Validate the structural correctness of a [`Recurrence`] rule against the
/// given `start_at`. Does not enforce the occurrence/span caps — those are
/// applied in [`expand_recurrence`] after generation.
fn validate_recurrence(start_at: NaiveDateTime, rule: &Recurrence) -> Result<(), RecurrenceError> {
    if rule.interval < 1 {
        return Err(RecurrenceError::InvalidInterval);
    }
    match (rule.count, rule.until) {
        (None, None) | (Some(_), Some(_)) => return Err(RecurrenceError::AmbiguousTerminator),
        _ => {}
    }
    if let Some(weekdays) = &rule.by_weekdays {
        if !matches!(rule.frequency, Frequency::Weekly | Frequency::Biweekly) {
            return Err(RecurrenceError::ByWeekdaysOnNonWeekly);
        }
        if weekdays.is_empty() {
            return Err(RecurrenceError::EmptyByWeekdays);
        }
        if !weekdays.contains(&start_at.weekday()) {
            return Err(RecurrenceError::StartWeekdayMismatch);
        }
    }
    Ok(())
}

/// Expand the rule into concrete occurrence datetimes, in chronological order.
///
/// Validates the rule, generates events according to the frequency, then enforces
/// [`MAX_RECURRING_OCCURRENCES`] and [`MAX_RECURRING_SPAN_DAYS`]. The first
/// occurrence is always equal to `start_at`.
pub fn expand_recurrence(
    start_at: NaiveDateTime,
    rule: &Recurrence,
) -> Result<Vec<NaiveDateTime>, RecurrenceError> {
    validate_recurrence(start_at, rule)?;

    let target_count = rule.count.map(|c| c as usize);
    let until = rule.until;

    // Generation cap: one past the limit so the outer check below sees overflow.
    let gen_cap = MAX_RECURRING_OCCURRENCES + 1;

    let week_interval = match rule.frequency {
        Frequency::Biweekly => 2 * rule.interval,
        _ => rule.interval,
    };

    let occurrences = match rule.frequency {
        Frequency::Daily => generate_stepped(
            start_at,
            Duration::days(rule.interval as i64),
            target_count,
            until,
            gen_cap,
        ),
        Frequency::Weekly | Frequency::Biweekly => match &rule.by_weekdays {
            Some(weekdays) => generate_weekly_with_weekdays(
                start_at,
                week_interval,
                weekdays,
                target_count,
                until,
                gen_cap,
            ),
            None => generate_stepped(
                start_at,
                Duration::weeks(week_interval as i64),
                target_count,
                until,
                gen_cap,
            ),
        },
        Frequency::Monthly => {
            generate_monthly(start_at, rule.interval, target_count, until, gen_cap)
        }
    };

    if occurrences.is_empty() {
        return Err(RecurrenceError::NoOccurrencesGenerated);
    }
    if occurrences.len() > MAX_RECURRING_OCCURRENCES {
        return Err(RecurrenceError::TooManyOccurrences {
            count: occurrences.len(),
        });
    }
    let span = occurrences
        .last()
        .unwrap()
        .signed_duration_since(*occurrences.first().unwrap());
    if span.num_days() > MAX_RECURRING_SPAN_DAYS {
        return Err(RecurrenceError::SpanTooLong {
            days: span.num_days(),
        });
    }
    Ok(occurrences)
}

/// Generic stepped generator for `daily` and `weekly`-without-`by_weekdays`.
fn generate_stepped(
    start_at: NaiveDateTime,
    step: Duration,
    target_count: Option<usize>,
    until: Option<NaiveDateTime>,
    gen_cap: usize,
) -> Vec<NaiveDateTime> {
    let mut out = Vec::new();
    let mut current = start_at;
    let limit = target_count.unwrap_or(gen_cap).min(gen_cap);
    while out.len() < limit {
        if let Some(end) = until {
            if current > end {
                break;
            }
        }
        out.push(current);
        current = match current.checked_add_signed(step) {
            Some(next) => next,
            None => break,
        };
    }
    out
}

/// Weekly recurrence with explicit weekdays. Walks `week_interval`-aligned weeks
/// starting from the Monday of `start_at`'s week and emits each listed weekday
/// (skipping any occurrence that falls before `start_at` itself).
fn generate_weekly_with_weekdays(
    start_at: NaiveDateTime,
    week_interval: u32,
    weekdays: &[Weekday],
    target_count: Option<usize>,
    until: Option<NaiveDateTime>,
    gen_cap: usize,
) -> Vec<NaiveDateTime> {
    let mut sorted: Vec<Weekday> = weekdays.to_vec();
    sorted.sort_by_key(|w| w.num_days_from_monday());
    sorted.dedup();

    let time_of_day = start_at.time();
    let monday_of_start_week =
        start_at.date() - Duration::days(start_at.weekday().num_days_from_monday() as i64);

    let mut out = Vec::new();
    let limit = target_count.unwrap_or(gen_cap).min(gen_cap);
    let mut week_offset: i64 = 0;
    // Safety cap on outer loop iterations to bound worst-case work even when
    // `until` is far in the future and weekday filters mean few hits per week.
    let max_weeks = (gen_cap as i64) + 8;
    while out.len() < limit && week_offset < max_weeks * (week_interval as i64) {
        let week_monday =
            match monday_of_start_week.checked_add_signed(Duration::days(week_offset * 7)) {
                Some(d) => d,
                None => break,
            };
        for wd in &sorted {
            let date = match week_monday
                .checked_add_signed(Duration::days(wd.num_days_from_monday() as i64))
            {
                Some(d) => d,
                None => return out,
            };
            let dt = date.and_time(time_of_day);
            if dt < start_at {
                continue;
            }
            if let Some(end) = until {
                if dt > end {
                    return out;
                }
            }
            out.push(dt);
            if out.len() >= limit {
                return out;
            }
        }
        week_offset += week_interval as i64;
    }
    out
}

/// Monthly recurrence. Day-of-month is taken from `start_at`; chrono's
/// `checked_add_months` automatically clamps to the last day of the target
/// month when the day doesn't exist (e.g. Jan 31 + 1 month → Feb 28).
fn generate_monthly(
    start_at: NaiveDateTime,
    interval: u32,
    target_count: Option<usize>,
    until: Option<NaiveDateTime>,
    gen_cap: usize,
) -> Vec<NaiveDateTime> {
    let mut out = Vec::new();
    let limit = target_count.unwrap_or(gen_cap).min(gen_cap);
    let mut i: u32 = 0;
    while out.len() < limit {
        let candidate = match start_at.checked_add_months(Months::new(i * interval)) {
            Some(dt) => dt,
            None => break,
        };
        if let Some(end) = until {
            if candidate > end {
                break;
            }
        }
        out.push(candidate);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_time(NaiveTime::from_hms_opt(h, min, 0).unwrap())
    }

    fn rule(frequency: Frequency) -> Recurrence {
        Recurrence {
            frequency,
            interval: 1,
            by_weekdays: None,
            count: Some(3),
            until: None,
        }
    }

    // ───── validate_recurrence ─────

    #[test]
    fn validate_rejects_zero_interval() {
        let mut r = rule(Frequency::Weekly);
        r.interval = 0;
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::InvalidInterval);
    }

    #[test]
    fn validate_rejects_missing_terminator() {
        let mut r = rule(Frequency::Weekly);
        r.count = None;
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::AmbiguousTerminator);
    }

    #[test]
    fn validate_rejects_both_count_and_until() {
        let mut r = rule(Frequency::Weekly);
        r.until = Some(dt(2026, 12, 1, 10, 0));
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::AmbiguousTerminator);
    }

    #[test]
    fn validate_rejects_by_weekdays_on_monthly() {
        let mut r = rule(Frequency::Monthly);
        r.by_weekdays = Some(vec![Weekday::Mon]);
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::ByWeekdaysOnNonWeekly);
    }

    #[test]
    fn validate_rejects_by_weekdays_on_daily() {
        let mut r = rule(Frequency::Daily);
        r.by_weekdays = Some(vec![Weekday::Mon]);
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::ByWeekdaysOnNonWeekly);
    }

    #[test]
    fn validate_rejects_empty_by_weekdays() {
        let mut r = rule(Frequency::Weekly);
        r.by_weekdays = Some(vec![]);
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::EmptyByWeekdays);
    }

    #[test]
    fn validate_rejects_start_weekday_not_in_by_weekdays() {
        // 2026-06-01 is a Monday; by_weekdays excludes it.
        let mut r = rule(Frequency::Weekly);
        r.by_weekdays = Some(vec![Weekday::Wed, Weekday::Fri]);
        let err = validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap_err();
        assert_eq!(err, RecurrenceError::StartWeekdayMismatch);
    }

    #[test]
    fn validate_accepts_when_start_weekday_matches() {
        let mut r = rule(Frequency::Weekly);
        r.by_weekdays = Some(vec![Weekday::Mon, Weekday::Wed]);
        // 2026-06-01 is a Monday.
        validate_recurrence(dt(2026, 6, 1, 10, 0), &r).unwrap();
    }

    // ───── expand_recurrence: daily ─────

    #[test]
    fn expand_daily_with_count() {
        let start = dt(2026, 6, 1, 9, 0);
        let r = Recurrence {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekdays: None,
            count: Some(4),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 9, 0),
                dt(2026, 6, 2, 9, 0),
                dt(2026, 6, 3, 9, 0),
                dt(2026, 6, 4, 9, 0),
            ]
        );
    }

    #[test]
    fn expand_daily_with_interval_3() {
        let start = dt(2026, 6, 1, 9, 0);
        let r = Recurrence {
            frequency: Frequency::Daily,
            interval: 3,
            by_weekdays: None,
            count: Some(3),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 9, 0),
                dt(2026, 6, 4, 9, 0),
                dt(2026, 6, 7, 9, 0),
            ]
        );
    }

    #[test]
    fn expand_daily_with_until() {
        let start = dt(2026, 6, 1, 9, 0);
        let r = Recurrence {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekdays: None,
            count: None,
            until: Some(dt(2026, 6, 3, 9, 0)),
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(occ.len(), 3);
        assert_eq!(occ.last().unwrap(), &dt(2026, 6, 3, 9, 0));
    }

    // ───── expand_recurrence: weekly without by_weekdays ─────

    #[test]
    fn expand_weekly_interval_1() {
        let start = dt(2026, 6, 1, 9, 0); // Monday
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(3),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 9, 0),
                dt(2026, 6, 8, 9, 0),
                dt(2026, 6, 15, 9, 0),
            ]
        );
    }

    #[test]
    fn expand_weekly_interval_2() {
        let start = dt(2026, 6, 1, 9, 0);
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 2,
            by_weekdays: None,
            count: Some(3),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 9, 0),
                dt(2026, 6, 15, 9, 0),
                dt(2026, 6, 29, 9, 0),
            ]
        );
    }

    // ───── expand_recurrence: weekly with by_weekdays ─────

    #[test]
    fn expand_weekly_mon_wed() {
        // Start Monday 2026-06-01, alternate Mon+Wed for 6 sessions = 3 weeks.
        let start = dt(2026, 6, 1, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: Some(vec![Weekday::Mon, Weekday::Wed]),
            count: Some(6),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 10, 0),  // Mon
                dt(2026, 6, 3, 10, 0),  // Wed
                dt(2026, 6, 8, 10, 0),  // Mon
                dt(2026, 6, 10, 10, 0), // Wed
                dt(2026, 6, 15, 10, 0), // Mon
                dt(2026, 6, 17, 10, 0), // Wed
            ]
        );
    }

    #[test]
    fn expand_weekly_tue_thu_fri_starting_thursday() {
        // Start on Thursday; only Thu+Fri of week 0 emit (not Tue, which is before start).
        let start = dt(2026, 6, 4, 10, 0); // Thursday
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: Some(vec![Weekday::Tue, Weekday::Thu, Weekday::Fri]),
            count: Some(5),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 4, 10, 0),  // Thu (start)
                dt(2026, 6, 5, 10, 0),  // Fri
                dt(2026, 6, 9, 10, 0),  // Tue
                dt(2026, 6, 11, 10, 0), // Thu
                dt(2026, 6, 12, 10, 0), // Fri
            ]
        );
    }

    #[test]
    fn expand_biweekly_with_by_weekdays() {
        // Biweekly Mon+Wed: pattern is Mon+Wed of week 0, skip week 1, Mon+Wed of week 2…
        let start = dt(2026, 6, 1, 10, 0); // Monday
        let r = Recurrence {
            frequency: Frequency::Biweekly,
            interval: 1,
            by_weekdays: Some(vec![Weekday::Mon, Weekday::Wed]),
            count: Some(4),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 1, 10, 0),  // Mon week 0
                dt(2026, 6, 3, 10, 0),  // Wed week 0
                dt(2026, 6, 15, 10, 0), // Mon week 2
                dt(2026, 6, 17, 10, 0), // Wed week 2
            ]
        );
    }

    // ───── expand_recurrence: monthly ─────

    #[test]
    fn expand_monthly_simple() {
        let start = dt(2026, 6, 15, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Monthly,
            interval: 1,
            by_weekdays: None,
            count: Some(3),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 15, 10, 0),
                dt(2026, 7, 15, 10, 0),
                dt(2026, 8, 15, 10, 0),
            ]
        );
    }

    #[test]
    fn expand_monthly_clamps_day_for_short_months() {
        // Jan 31 → Feb 28 (2026 is not a leap year) → Mar 31 → Apr 30.
        let start = dt(2026, 1, 31, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Monthly,
            interval: 1,
            by_weekdays: None,
            count: Some(4),
            until: None,
        };
        let occ = expand_recurrence(start, &r).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 1, 31, 10, 0),
                dt(2026, 2, 28, 10, 0),
                dt(2026, 3, 31, 10, 0),
                dt(2026, 4, 30, 10, 0),
            ]
        );
    }

    // ───── expand_recurrence: cap enforcement ─────

    #[test]
    fn expand_rejects_count_exceeding_max_occurrences() {
        let start = dt(2026, 1, 1, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Daily,
            interval: 1,
            by_weekdays: None,
            count: Some((MAX_RECURRING_OCCURRENCES as u32) + 5),
            until: None,
        };
        let err = expand_recurrence(start, &r).unwrap_err();
        assert!(matches!(err, RecurrenceError::TooManyOccurrences { .. }));
    }

    #[test]
    fn expand_rejects_count_producing_too_long_span() {
        // Weekly for 60 weeks: 60*7 = 420 days, exceeds MAX_RECURRING_SPAN_DAYS (366).
        let start = dt(2026, 1, 1, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: Some(60),
            until: None,
        };
        let err = expand_recurrence(start, &r).unwrap_err();
        assert!(matches!(err, RecurrenceError::SpanTooLong { .. }));
    }

    #[test]
    fn expand_rejects_until_producing_too_long_span() {
        // Weekly with `until` ~14 months out: produces 61 occurrences (well under
        // the 365-occurrence cap) but spans 420 days, exceeding the 366-day cap.
        // Exercises the span cap via the `until` terminator path.
        let start = dt(2026, 1, 1, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: None,
            until: Some(dt(2027, 3, 1, 10, 0)),
        };
        let err = expand_recurrence(start, &r).unwrap_err();
        assert!(matches!(err, RecurrenceError::SpanTooLong { .. }));
    }

    #[test]
    fn expand_rejects_until_in_past_of_start() {
        let start = dt(2026, 6, 1, 10, 0);
        let r = Recurrence {
            frequency: Frequency::Weekly,
            interval: 1,
            by_weekdays: None,
            count: None,
            until: Some(dt(2026, 5, 1, 10, 0)),
        };
        let err = expand_recurrence(start, &r).unwrap_err();
        assert_eq!(err, RecurrenceError::NoOccurrencesGenerated);
    }
}
