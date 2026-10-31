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
//! Expansion runs on local wall time in the series' zone and resolves each slot to UTC,
//! the way RFC 5545 clients expand the `DTSTART;TZID` + `RRULE` the invite carries, so
//! the stored sessions and the calendar's instances stay the same instants across DST.

use chrono::{Datelike, Duration, Months, NaiveDateTime, Offset, TimeZone, Weekday};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{DomainErrorKind, Error};

/// Maximum number of occurrences a single recurrence rule may produce.
const MAX_RECURRING_OCCURRENCES: usize = 365;

/// Maximum calendar span (in days) from the first occurrence to the last.
const MAX_RECURRING_SPAN_DAYS: i64 = 366;

/// Latest day of the month a monthly series may start on. Clients skip months without
/// the start day while some clamp to the month's end, so later days cannot render alike.
const MAX_MONTHLY_START_DAY: u32 = 28;

/// How far back to look for the offset in effect before a spring-forward gap.
const GAP_LOOKBACK_HOURS: i64 = 3;

/// How often the rule repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Daily,
    Weekly,
    Biweekly,
    Monthly,
}

/// A recurrence rule. Exactly one of `count` or `until` must be specified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Recurrence {
    pub frequency: Frequency,
    /// Step multiplier. `weekly` with `interval: 2` is the same as `biweekly`.
    /// Defaults to 1 if omitted in JSON.
    #[serde(default = "default_interval")]
    pub interval: u32,
    /// Only meaningful for `weekly`/`biweekly`. If present, the rule emits one
    /// event per listed weekday within each active week.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<Vec<String>>)]
    pub by_weekdays: Option<Vec<Weekday>>,
    /// Stop after this many occurrences. Mutually exclusive with `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Stop after this datetime. Mutually exclusive with `count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<NaiveDateTime>,
}

impl Recurrence {
    /// The step count in the frequency's own unit. `Biweekly` steps two weeks per
    /// `interval`. Shared with the `RRULE` builder so the emitted calendar rule and
    /// the human summary can never disagree about how often a series repeats.
    pub(crate) fn effective_interval(&self) -> u32 {
        match self.frequency {
            Frequency::Biweekly => 2 * self.interval,
            _ => self.interval,
        }
    }

    /// Human phrase for how often the rule repeats, e.g. `Every 2 weeks on Monday and
    /// Wednesday`. Terminators (`count`, `until`) are deliberately left out.
    pub fn summary(&self) -> String {
        let cadence = match (self.frequency, self.effective_interval()) {
            (Frequency::Daily, 1) => "Daily".to_string(),
            (Frequency::Daily, n) => format!("Every {n} days"),
            (Frequency::Weekly | Frequency::Biweekly, 1) => "Weekly".to_string(),
            (Frequency::Weekly | Frequency::Biweekly, n) => format!("Every {n} weeks"),
            (Frequency::Monthly, 1) => "Monthly".to_string(),
            (Frequency::Monthly, n) => format!("Every {n} months"),
        };
        match self.by_weekdays.as_deref() {
            Some([]) | None => cadence,
            Some(days) => format!("{cadence} on {}", join_weekdays(days)),
        }
    }
}

/// Full English weekday name.
fn weekday_name(wd: Weekday) -> &'static str {
    match wd {
        Weekday::Mon => "Monday",
        Weekday::Tue => "Tuesday",
        Weekday::Wed => "Wednesday",
        Weekday::Thu => "Thursday",
        Weekday::Fri => "Friday",
        Weekday::Sat => "Saturday",
        Weekday::Sun => "Sunday",
    }
}

/// `Monday`, `Monday and Wednesday`, `Monday, Wednesday and Friday`.
fn join_weekdays(days: &[Weekday]) -> String {
    let names: Vec<&str> = days.iter().map(|d| weekday_name(*d)).collect();
    match names.split_last() {
        Some((last, [])) => last.to_string(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
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
    MonthlyStartDayTooLate,
    AmbiguousStartAt,
    TooManyOccurrences { count: usize },
    SpanTooLong { days: i64 },
    NoOccurrencesGenerated,
    StartAtInPast,
}

impl From<RecurrenceError> for Error {
    fn from(err: RecurrenceError) -> Self {
        Self {
            source: None,
            error_kind: DomainErrorKind::Validation(err.message()),
        }
    }
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
            Self::MonthlyStartDayTooLate => format!(
                "Monthly series must start on day {MAX_MONTHLY_START_DAY} or earlier in the coach's timezone."
            ),
            Self::AmbiguousStartAt => {
                "`start_at` is the repeated hour of a daylight saving change; choose another time"
                    .into()
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
            Self::StartAtInPast => "`start_at` must not be in the past".into(),
        }
    }
}

/// The checks a rule needs to be expanded at all. Does not enforce the occurrence/span
/// caps; those are applied after generation.
fn validate_structure(rule: &Recurrence) -> Result<(), RecurrenceError> {
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
    }
    Ok(())
}

/// Validate a rule about to be published against the series' local start: its
/// structure, plus the rules that keep a new series rendering alike in every client.
fn validate_recurrence(
    local_start: NaiveDateTime,
    rule: &Recurrence,
) -> Result<(), RecurrenceError> {
    validate_structure(rule)?;
    if rule
        .by_weekdays
        .as_ref()
        .is_some_and(|weekdays| !weekdays.contains(&local_start.weekday()))
    {
        return Err(RecurrenceError::StartWeekdayMismatch);
    }
    if rule.frequency == Frequency::Monthly && local_start.day() > MAX_MONTHLY_START_DAY {
        return Err(RecurrenceError::MonthlyStartDayTooLate);
    }
    Ok(())
}

/// The wall time `utc` shows in `zone`.
fn to_local(zone: Tz, utc: NaiveDateTime) -> NaiveDateTime {
    zone.from_utc_datetime(&utc).naive_local()
}

/// The UTC instant of a local wall time, resolved as RFC 5545 §3.3.5 does: a repeated
/// time is its first occurrence, and a skipped time uses the offset before the gap.
fn to_utc(zone: Tz, local: NaiveDateTime) -> NaiveDateTime {
    zone.from_local_datetime(&local)
        .earliest()
        .map(|resolved| resolved.naive_utc())
        .unwrap_or_else(|| local - offset_before_gap(zone, local))
}

/// The UTC offset in effect just before the gap that swallowed `local`.
fn offset_before_gap(zone: Tz, local: NaiveDateTime) -> Duration {
    let before = local - Duration::hours(GAP_LOOKBACK_HOURS);
    let seconds = zone
        .from_local_datetime(&before)
        .earliest()
        .map_or(0, |resolved| resolved.offset().fix().local_minus_utc());
    Duration::seconds(seconds.into())
}

/// Expand a rule about to be published into concrete occurrence datetimes (UTC), in
/// chronological order.
///
/// Steps run on local wall time in `zone`, so every slot keeps the start's local time of
/// day across DST, and weekdays and month days are the ones the coach sees. `until` is
/// compared against each resolved instant, as a client reads `UNTIL`. Validates the rule,
/// generates events according to the frequency, then enforces
/// [`MAX_RECURRING_OCCURRENCES`] and [`MAX_RECURRING_SPAN_DAYS`]. The first occurrence
/// is always equal to `start_at`.
pub fn expand_recurrence(
    start_at: NaiveDateTime,
    rule: &Recurrence,
    zone: Tz,
) -> Result<Vec<NaiveDateTime>, RecurrenceError> {
    let local_start = to_local(zone, start_at);
    validate_recurrence(local_start, rule)?;
    // The second pass through a repeated hour: the invite's `DTSTART;TZID` would name the first.
    if to_utc(zone, local_start) != start_at {
        return Err(RecurrenceError::AmbiguousStartAt);
    }
    generate(local_start, rule, zone)
}

/// Expand a rule that is already published, into the instances its invite produced.
///
/// Skips the checks only a new rule must pass (start weekday, monthly start day, a start
/// in a repeated hour): a series stored before they existed still has to resolve, the
/// way a calendar resolves its `RRULE`, or it could never be resent.
pub(crate) fn expand_published(
    start_at: NaiveDateTime,
    rule: &Recurrence,
    zone: Tz,
) -> Result<Vec<NaiveDateTime>, RecurrenceError> {
    validate_structure(rule)?;
    generate(to_local(zone, start_at), rule, zone)
}

/// Generate the slots from the local start, then enforce the occurrence and span caps.
fn generate(
    local_start: NaiveDateTime,
    rule: &Recurrence,
    zone: Tz,
) -> Result<Vec<NaiveDateTime>, RecurrenceError> {
    let bounds = Bounds {
        resolve: |local| to_utc(zone, local),
        until: rule.until,
        // One past the limit so the outer check below sees overflow.
        limit: rule
            .count
            .map_or(MAX_RECURRING_OCCURRENCES + 1, |c| c as usize)
            .min(MAX_RECURRING_OCCURRENCES + 1),
    };

    let occurrences = match rule.frequency {
        Frequency::Daily => {
            generate_stepped(local_start, Duration::days(rule.interval as i64), &bounds)
        }
        Frequency::Weekly | Frequency::Biweekly => match &rule.by_weekdays {
            Some(weekdays) => generate_weekly_with_weekdays(
                local_start,
                rule.effective_interval(),
                weekdays,
                &bounds,
            ),
            None => generate_stepped(
                local_start,
                Duration::weeks(rule.effective_interval() as i64),
                &bounds,
            ),
        },
        Frequency::Monthly => generate_monthly(local_start, rule.interval, &bounds),
    };

    let (Some(first), Some(last)) = (occurrences.first(), occurrences.last()) else {
        return Err(RecurrenceError::NoOccurrencesGenerated);
    };
    if occurrences.len() > MAX_RECURRING_OCCURRENCES {
        return Err(RecurrenceError::TooManyOccurrences {
            count: occurrences.len(),
        });
    }
    let span_days = last.signed_duration_since(*first).num_days();
    if span_days > MAX_RECURRING_SPAN_DAYS {
        return Err(RecurrenceError::SpanTooLong { days: span_days });
    }
    Ok(occurrences)
}

/// Shared limits for the generators: how a local slot becomes an instant, the `until`
/// instant it must not pass, and how many slots to emit at most.
struct Bounds<F: Fn(NaiveDateTime) -> NaiveDateTime> {
    resolve: F,
    until: Option<NaiveDateTime>,
    limit: usize,
}

impl<F: Fn(NaiveDateTime) -> NaiveDateTime> Bounds<F> {
    /// The slot's instant, or `None` once it is past `until`.
    fn admit(&self, local: NaiveDateTime) -> Option<NaiveDateTime> {
        let instant = (self.resolve)(local);
        self.until
            .is_none_or(|end| instant <= end)
            .then_some(instant)
    }
}

/// Generic stepped generator for `daily` and `weekly`-without-`by_weekdays`.
fn generate_stepped<F: Fn(NaiveDateTime) -> NaiveDateTime>(
    local_start: NaiveDateTime,
    step: Duration,
    bounds: &Bounds<F>,
) -> Vec<NaiveDateTime> {
    std::iter::successors(Some(local_start), |local| local.checked_add_signed(step))
        .map_while(|local| bounds.admit(local))
        .take(bounds.limit)
        .collect()
}

/// Weekly recurrence with explicit weekdays. Walks `week_interval`-aligned weeks
/// starting from the Monday of the start's week (RFC 5545 `WKST=MO`) and emits each
/// listed weekday, skipping any occurrence that falls before the start itself.
fn generate_weekly_with_weekdays<F: Fn(NaiveDateTime) -> NaiveDateTime>(
    local_start: NaiveDateTime,
    week_interval: u32,
    weekdays: &[Weekday],
    bounds: &Bounds<F>,
) -> Vec<NaiveDateTime> {
    let mut sorted: Vec<Weekday> = weekdays.to_vec();
    sorted.sort_by_key(|w| w.num_days_from_monday());
    sorted.dedup();

    let time_of_day = local_start.time();
    let monday_of_start_week =
        local_start.date() - Duration::days(local_start.weekday().num_days_from_monday() as i64);
    // Bounds worst-case work when `until` is far out and the weekday filter hits rarely.
    let max_weeks = (bounds.limit as i64) + 8;

    (0..max_weeks)
        .map_while(|week| {
            monday_of_start_week.checked_add_signed(Duration::weeks(week * week_interval as i64))
        })
        .flat_map(|monday| {
            sorted.iter().filter_map(move |wd| {
                monday.checked_add_signed(Duration::days(wd.num_days_from_monday() as i64))
            })
        })
        .map(|date| date.and_time(time_of_day))
        .filter(|local| *local >= local_start)
        .map_while(|local| bounds.admit(local))
        .take(bounds.limit)
        .collect()
}

/// Monthly recurrence on the start's day of the month. A month without that day is
/// skipped, as RFC 5545 skips it; only a stored rule can reach one, since new rules
/// start on day [`MAX_MONTHLY_START_DAY`] or earlier.
fn generate_monthly<F: Fn(NaiveDateTime) -> NaiveDateTime>(
    local_start: NaiveDateTime,
    interval: u32,
    bounds: &Bounds<F>,
) -> Vec<NaiveDateTime> {
    (0u32..)
        .map_while(|i| {
            i.checked_mul(interval)
                .and_then(|months| local_start.checked_add_months(Months::new(months)))
        })
        // `checked_add_months` clamps to the month's end; a clamped day is no occurrence.
        .filter(|local| local.day() == local_start.day())
        .map_while(|local| bounds.admit(local))
        .take(bounds.limit)
        .collect()
}

#[cfg(test)]
#[path = "recurrence_dst_tests.rs"]
mod dst_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};
    use chrono_tz::UTC;

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

    /// A rule with the given frequency and interval, no weekdays.
    fn cadence(frequency: Frequency, interval: u32) -> Recurrence {
        Recurrence {
            frequency,
            interval,
            ..rule(frequency)
        }
    }

    // ───── summary ─────

    #[test]
    fn summary_daily() {
        assert_eq!(cadence(Frequency::Daily, 1).summary(), "Daily");
        assert_eq!(cadence(Frequency::Daily, 2).summary(), "Every 2 days");
        assert_eq!(cadence(Frequency::Daily, 5).summary(), "Every 5 days");
    }

    #[test]
    fn summary_weekly() {
        assert_eq!(cadence(Frequency::Weekly, 1).summary(), "Weekly");
        assert_eq!(cadence(Frequency::Weekly, 2).summary(), "Every 2 weeks");
        assert_eq!(cadence(Frequency::Weekly, 3).summary(), "Every 3 weeks");
    }

    #[test]
    fn summary_biweekly_doubles_the_interval() {
        assert_eq!(cadence(Frequency::Biweekly, 1).summary(), "Every 2 weeks");
        assert_eq!(cadence(Frequency::Biweekly, 2).summary(), "Every 4 weeks");
    }

    #[test]
    fn summary_monthly() {
        assert_eq!(cadence(Frequency::Monthly, 1).summary(), "Monthly");
        assert_eq!(cadence(Frequency::Monthly, 2).summary(), "Every 2 months");
        assert_eq!(cadence(Frequency::Monthly, 6).summary(), "Every 6 months");
    }

    /// The same cadence spelled two ways must read identically.
    #[test]
    fn summary_biweekly_matches_weekly_interval_2() {
        assert_eq!(
            cadence(Frequency::Biweekly, 1).summary(),
            cadence(Frequency::Weekly, 2).summary()
        );
    }

    #[test]
    fn summary_appends_weekdays_in_listed_order() {
        let one = Recurrence {
            by_weekdays: Some(vec![Weekday::Mon]),
            ..cadence(Frequency::Weekly, 1)
        };
        assert_eq!(one.summary(), "Weekly on Monday");

        let two = Recurrence {
            by_weekdays: Some(vec![Weekday::Mon, Weekday::Wed]),
            ..cadence(Frequency::Weekly, 1)
        };
        assert_eq!(two.summary(), "Weekly on Monday and Wednesday");

        let three = Recurrence {
            by_weekdays: Some(vec![Weekday::Wed, Weekday::Mon, Weekday::Fri]),
            ..cadence(Frequency::Weekly, 2)
        };
        assert_eq!(
            three.summary(),
            "Every 2 weeks on Wednesday, Monday and Friday"
        );
    }

    /// An empty list carries no information, so the cadence stands alone.
    #[test]
    fn summary_ignores_empty_weekdays() {
        let r = Recurrence {
            by_weekdays: Some(vec![]),
            ..cadence(Frequency::Weekly, 1)
        };
        assert_eq!(r.summary(), "Weekly");
    }

    /// Terminators belong to other template variables, never to the cadence phrase.
    #[test]
    fn summary_omits_terminators() {
        let counted = Recurrence {
            count: Some(12),
            ..cadence(Frequency::Weekly, 1)
        };
        assert_eq!(counted.summary(), "Weekly");

        let dated = Recurrence {
            count: None,
            until: Some(dt(2026, 12, 1, 10, 0)),
            ..cadence(Frequency::Weekly, 1)
        };
        assert_eq!(dated.summary(), "Weekly");
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
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
        let occ = expand_recurrence(start, &r, UTC).unwrap();
        assert_eq!(
            occ,
            vec![
                dt(2026, 6, 15, 10, 0),
                dt(2026, 7, 15, 10, 0),
                dt(2026, 8, 15, 10, 0),
            ]
        );
    }

    /// Clients disagree on months without the start day, so such a series cannot exist.
    #[test]
    fn expand_monthly_rejects_start_days_after_28() {
        let r = Recurrence {
            frequency: Frequency::Monthly,
            interval: 1,
            by_weekdays: None,
            count: Some(4),
            until: None,
        };
        [29, 30, 31].into_iter().for_each(|day| {
            let err = expand_recurrence(dt(2026, 1, day, 10, 0), &r, UTC).unwrap_err();
            assert_eq!(err, RecurrenceError::MonthlyStartDayTooLate, "day {day}");
        });
        assert_eq!(
            expand_recurrence(dt(2026, 1, 28, 10, 0), &r, UTC).unwrap(),
            vec![
                dt(2026, 1, 28, 10, 0),
                dt(2026, 2, 28, 10, 0),
                dt(2026, 3, 28, 10, 0),
                dt(2026, 4, 28, 10, 0),
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
        let err = expand_recurrence(start, &r, UTC).unwrap_err();
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
        let err = expand_recurrence(start, &r, UTC).unwrap_err();
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
        let err = expand_recurrence(start, &r, UTC).unwrap_err();
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
        let err = expand_recurrence(start, &r, UTC).unwrap_err();
        assert_eq!(err, RecurrenceError::NoOccurrencesGenerated);
    }
}
