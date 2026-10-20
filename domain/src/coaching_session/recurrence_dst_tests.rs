//! Expansion in the series' zone: wall time held across DST, weekdays and month days
//! read locally, and the RFC 5545 rules for skipped and repeated local times.

use chrono::{Days, NaiveDate};
use chrono_tz::America::Chicago;
use chrono_tz::Asia::{Kolkata, Tokyo};
use chrono_tz::Australia::Sydney;
use chrono_tz::Europe::London;
use chrono_tz::UTC;

use super::*;

fn wall(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, min, 0)
        .unwrap()
}

/// The instant of an unambiguous local wall time.
fn at(zone: Tz, y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    zone.from_local_datetime(&wall(y, m, d, h, min))
        .single()
        .unwrap()
        .naive_utc()
}

fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    wall(y, m, d, h, min)
}

fn counted(frequency: Frequency, count: u32) -> Recurrence {
    Recurrence {
        frequency,
        interval: 1,
        by_weekdays: None,
        count: Some(count),
        until: None,
    }
}

fn on(weekdays: &[Weekday], frequency: Frequency, count: u32) -> Recurrence {
    Recurrence {
        by_weekdays: Some(weekdays.to_vec()),
        ..counted(frequency, count)
    }
}

/// The `k`th local slot of a rule with no weekday filter.
fn nth_local(start: NaiveDateTime, frequency: Frequency, k: u32) -> NaiveDateTime {
    let date = match frequency {
        Frequency::Daily => start.date() + Days::new(k.into()),
        Frequency::Weekly => start.date() + Days::new(7 * u64::from(k)),
        Frequency::Biweekly => start.date() + Days::new(14 * u64::from(k)),
        Frequency::Monthly => start.date().checked_add_months(Months::new(k)).unwrap(),
    };
    date.and_time(start.time())
}

/// Every frequency, started shortly before a transition, keeps its local time of day and
/// local date step on both sides of it, in zones north and south of the equator.
#[test]
fn every_frequency_keeps_wall_time_across_both_transitions() {
    let starts = [
        (Chicago, wall(2026, 10, 20, 10, 0)),
        (Chicago, wall(2027, 3, 2, 10, 0)),
        (London, wall(2026, 10, 13, 10, 0)),
        (London, wall(2027, 3, 16, 10, 0)),
        (Sydney, wall(2027, 3, 23, 10, 0)),
        (Sydney, wall(2027, 9, 21, 10, 0)),
        (Kolkata, wall(2026, 10, 20, 10, 0)),
        (UTC, wall(2026, 10, 20, 10, 0)),
    ];
    let cadences = [
        (Frequency::Daily, 30),
        (Frequency::Weekly, 6),
        (Frequency::Biweekly, 4),
        (Frequency::Monthly, 3),
    ];

    for (zone, local_start) in starts {
        for (frequency, count) in cadences {
            let start = zone.from_local_datetime(&local_start).unwrap().naive_utc();
            let slots = expand_recurrence(start, &counted(frequency, count), zone).unwrap();

            let expected: Vec<NaiveDateTime> = (0..count)
                .map(|k| to_utc(zone, nth_local(local_start, frequency, k)))
                .collect();
            assert_eq!(slots, expected, "{zone} {frequency:?} from {local_start}");
            assert!(
                slots
                    .iter()
                    .all(|slot| to_local(zone, *slot).time() == local_start.time()),
                "{zone} {frequency:?} drifted off {local_start}"
            );
        }
    }
}

/// The production series behind the report: weekly Thursdays at 11:00 Chicago through
/// the year. After Nov 1 the slot is 17:00Z, where the calendar holds it, not 16:00Z.
#[test]
fn chicago_weekly_series_lands_where_the_calendar_holds_it() {
    let r = Recurrence {
        until: Some(utc(2027, 1, 1, 5, 59)),
        count: None,
        ..on(&[Weekday::Thu], Frequency::Weekly, 1)
    };
    let slots = expand_recurrence(utc(2026, 9, 10, 16, 0), &r, Chicago).unwrap();

    assert_eq!(slots.len(), 17);
    assert_eq!(slots[7], utc(2026, 10, 29, 16, 0));
    assert_eq!(slots[8], utc(2026, 11, 5, 17, 0));
    assert!(slots.iter().all(|slot| {
        let local = to_local(Chicago, *slot);
        local.weekday() == Weekday::Thu && local.time() == wall(2026, 1, 1, 11, 0).time()
    }));
}

/// Thursday 7 PM Chicago is Friday in UTC. The weekday is the coach's, so this is
/// accepted and stays on Thursday evenings.
#[test]
fn evening_weekday_is_read_in_the_series_zone() {
    let start = at(Chicago, 2026, 10, 22, 19, 0);
    assert_eq!(start.weekday(), Weekday::Fri);

    let slots =
        expand_recurrence(start, &on(&[Weekday::Thu], Frequency::Weekly, 4), Chicago).unwrap();
    assert_eq!(
        slots,
        vec![
            at(Chicago, 2026, 10, 22, 19, 0),
            at(Chicago, 2026, 10, 29, 19, 0),
            at(Chicago, 2026, 11, 5, 19, 0),
            at(Chicago, 2026, 11, 12, 19, 0),
        ]
    );
}

/// Picking the UTC weekday as well used to slip the other day back by one locally.
#[test]
fn evening_weekdays_stay_on_the_chosen_local_days() {
    let start = at(Chicago, 2026, 10, 22, 19, 0);
    let rule = on(&[Weekday::Thu, Weekday::Fri], Frequency::Weekly, 4);

    let slots = expand_recurrence(start, &rule, Chicago).unwrap();
    assert_eq!(
        slots,
        vec![
            at(Chicago, 2026, 10, 22, 19, 0),
            at(Chicago, 2026, 10, 23, 19, 0),
            at(Chicago, 2026, 10, 29, 19, 0),
            at(Chicago, 2026, 10, 30, 19, 0),
        ]
    );
}

/// Monday 08:00 in Tokyo is Sunday in UTC.
#[test]
fn morning_weekday_ahead_of_utc_is_read_in_the_series_zone() {
    let start = at(Tokyo, 2026, 10, 19, 8, 0);
    assert_eq!(start.weekday(), Weekday::Sun);

    let slots =
        expand_recurrence(start, &on(&[Weekday::Mon], Frequency::Weekly, 3), Tokyo).unwrap();
    assert!(slots
        .iter()
        .all(|slot| to_local(Tokyo, *slot).weekday() == Weekday::Mon));
}

/// The start weekday is still required, in local terms.
#[test]
fn start_weekday_mismatch_is_judged_locally() {
    let start = at(Chicago, 2026, 10, 22, 19, 0);
    let err =
        expand_recurrence(start, &on(&[Weekday::Fri], Frequency::Weekly, 4), Chicago).unwrap_err();
    assert_eq!(err, RecurrenceError::StartWeekdayMismatch);
}

/// Weeks are Monday-aligned, so a biweekly Sunday belongs to the week before it.
#[test]
fn biweekly_with_sunday_aligns_weeks_on_monday() {
    let start = utc(2027, 1, 4, 10, 0);
    let rule = on(&[Weekday::Mon, Weekday::Sun], Frequency::Biweekly, 4);

    assert_eq!(
        expand_recurrence(start, &rule, UTC).unwrap(),
        vec![
            utc(2027, 1, 4, 10, 0),
            utc(2027, 1, 10, 10, 0),
            utc(2027, 1, 18, 10, 0),
            utc(2027, 1, 24, 10, 0),
        ]
    );
}

/// The day of the month is the coach's: the 28th at 7 PM Chicago is the 29th in UTC and
/// is allowed, while the 29th at 8 AM Tokyo is the 28th in UTC and is not.
#[test]
fn monthly_start_day_is_judged_locally() {
    let start = at(Chicago, 2027, 1, 28, 19, 0);
    assert_eq!(start.day(), 29);
    assert_eq!(
        expand_recurrence(start, &counted(Frequency::Monthly, 3), Chicago).unwrap(),
        vec![
            at(Chicago, 2027, 1, 28, 19, 0),
            at(Chicago, 2027, 2, 28, 19, 0),
            at(Chicago, 2027, 3, 28, 19, 0),
        ]
    );

    let start = at(Tokyo, 2027, 1, 29, 8, 0);
    assert_eq!(start.day(), 28);
    assert_eq!(
        expand_recurrence(start, &counted(Frequency::Monthly, 3), Tokyo).unwrap_err(),
        RecurrenceError::MonthlyStartDayTooLate
    );
}

/// A skipped local time uses the offset before the gap: 02:30 on the spring-forward day
/// is 03:30 daylight time.
#[test]
fn skipped_local_time_uses_the_offset_before_the_gap() {
    let start = at(Chicago, 2027, 3, 12, 2, 30);
    let slots = expand_recurrence(start, &counted(Frequency::Daily, 4), Chicago).unwrap();
    assert_eq!(
        slots,
        vec![
            utc(2027, 3, 12, 8, 30),
            utc(2027, 3, 13, 8, 30),
            utc(2027, 3, 14, 8, 30),
            utc(2027, 3, 15, 7, 30),
        ]
    );
    assert_eq!(
        to_local(Chicago, slots[2]),
        wall(2027, 3, 14, 3, 30),
        "the skipped slot lands an hour later on the wall clock"
    );
}

/// A repeated local time is its first occurrence.
#[test]
fn repeated_local_time_is_its_first_occurrence() {
    let start = at(Chicago, 2026, 10, 30, 1, 30);
    let slots = expand_recurrence(start, &counted(Frequency::Daily, 4), Chicago).unwrap();
    assert_eq!(
        slots,
        vec![
            utc(2026, 10, 30, 6, 30),
            utc(2026, 10, 31, 6, 30),
            utc(2026, 11, 1, 6, 30),
            utc(2026, 11, 2, 7, 30),
        ]
    );
}

/// 01:30 CST on Nov 1 is the second pass through 01:30. Its `DTSTART;TZID` would be read
/// as the first, so the series cannot start there; the first pass is fine.
#[test]
fn start_in_the_second_pass_of_a_repeated_hour_is_rejected() {
    let second_pass = utc(2026, 11, 1, 7, 30);
    assert_eq!(
        expand_recurrence(second_pass, &counted(Frequency::Weekly, 3), Chicago).unwrap_err(),
        RecurrenceError::AmbiguousStartAt
    );

    let first_pass = utc(2026, 11, 1, 6, 30);
    let slots = expand_recurrence(first_pass, &counted(Frequency::Weekly, 3), Chicago).unwrap();
    assert_eq!(slots[0], first_pass);
}

/// `until` bounds the resolved instant: the post-DST slot is 17:00Z, so an `until` of
/// 16:59:59 excludes it.
#[test]
fn until_is_compared_against_the_resolved_instant() {
    let start = at(Chicago, 2026, 10, 29, 11, 0);
    let through = |until| Recurrence {
        until: Some(until),
        count: None,
        ..counted(Frequency::Weekly, 1)
    };

    let included = expand_recurrence(start, &through(utc(2026, 11, 5, 17, 0)), Chicago).unwrap();
    assert_eq!(included, vec![start, utc(2026, 11, 5, 17, 0)]);

    let cutoff = utc(2026, 11, 5, 17, 0) - Duration::seconds(1);
    let excluded = expand_recurrence(start, &through(cutoff), Chicago).unwrap();
    assert_eq!(excluded, vec![start]);
}

/// UTC has no transitions, so expansion there is exactly the old naive stepping.
#[test]
fn utc_expansion_is_plain_stepping() {
    let start = utc(2026, 10, 20, 15, 0);
    let slots = expand_recurrence(start, &counted(Frequency::Weekly, 4), UTC).unwrap();
    assert_eq!(
        slots,
        (0..4)
            .map(|k| start + Duration::weeks(k))
            .collect::<Vec<_>>()
    );
}

/// A stored rule from before the 29-31 rule still expands, the way a client reads it:
/// months without the start day are skipped, never clamped to the month's end.
#[test]
fn stored_late_month_rule_skips_short_months() {
    let start = utc(2026, 1, 31, 10, 0);
    let r = counted(Frequency::Monthly, 4);

    assert_eq!(
        expand_recurrence(start, &r, UTC).unwrap_err(),
        RecurrenceError::MonthlyStartDayTooLate
    );
    assert_eq!(
        expand_published(start, &r, UTC).unwrap(),
        vec![
            utc(2026, 1, 31, 10, 0),
            utc(2026, 3, 31, 10, 0),
            utc(2026, 5, 31, 10, 0),
            utc(2026, 7, 31, 10, 0),
        ]
    );
}

/// A stored start in the second pass of a repeated hour resolves as its `DTSTART;TZID`
/// does, to the first pass, instead of failing.
#[test]
fn stored_start_in_a_repeated_hour_resolves_to_the_first_pass() {
    let second_pass = utc(2026, 11, 1, 7, 30);
    let slots = expand_published(second_pass, &counted(Frequency::Weekly, 2), Chicago).unwrap();
    assert_eq!(
        slots,
        vec![utc(2026, 11, 1, 6, 30), utc(2026, 11, 8, 7, 30)]
    );
}

/// A stored evening rule accepted under the old UTC weekday check, whose local start
/// day is not listed, still expands instead of failing.
#[test]
fn stored_rule_whose_local_start_weekday_is_unlisted_still_expands() {
    let start = at(Chicago, 2026, 10, 22, 19, 0);
    let rule = on(&[Weekday::Fri], Frequency::Weekly, 2);

    assert_eq!(
        expand_recurrence(start, &rule, Chicago).unwrap_err(),
        RecurrenceError::StartWeekdayMismatch
    );
    assert_eq!(
        expand_published(start, &rule, Chicago).unwrap(),
        vec![
            at(Chicago, 2026, 10, 23, 19, 0),
            at(Chicago, 2026, 10, 30, 19, 0),
        ]
    );
}
