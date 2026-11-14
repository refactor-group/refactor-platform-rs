//! The series' stored slots must be the instances a calendar client expands from the
//! invite. An independent RFC 5545 implementation reads the exact `DTSTART` and `RRULE`
//! this builder emits, and must produce the same instants `expand_recurrence` does.

use chrono::{NaiveDate, Weekday};
use chrono_tz::America::Chicago;
use chrono_tz::Asia::{Kolkata, Tokyo};
use chrono_tz::Australia::Sydney;
use chrono_tz::Europe::London;
use rrule::RRuleSet;

use super::*;
use crate::coaching_session::recurrence::expand_published;
use crate::coaching_session::{expand_recurrence, Frequency, Recurrence};

fn wall(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, min, 0)
        .unwrap()
}

/// The instant of a local wall time, first pass when it repeats.
fn at(zone: chrono_tz::Tz, local: NaiveDateTime) -> NaiveDateTime {
    zone.from_local_datetime(&local)
        .earliest()
        .unwrap()
        .naive_utc()
}

fn series_invite(
    zone: chrono_tz::Tz,
    start: NaiveDateTime,
    rule: Recurrence,
) -> IcsInvite<'static> {
    IcsInvite {
        uid: "series-1@myrefactor.com".into(),
        sequence: 0,
        method: Method::Request,
        status: EventStatus::Confirmed,
        summary: "Jane / Alex".into(),
        description: "desc".into(),
        anchor_tz: zone,
        dtstamp: wall(2026, 10, 1, 12, 0),
        start,
        duration_minutes: 60,
        organizer: Participant::new("Refactor Coach", "hello@mail.myrefactor.com"),
        attendees: vec![Participant::new("Coachee Quinn", "coachee@example.com")],
        location_url: None,
        recurrence: Some(rule),
        recurrence_id: None,
        exdates: Vec::new(),
    }
}

/// The `DTSTART` and `RRULE` lines of the emitted VEVENT, as a client reads them.
fn emitted_rule(ics: &str) -> String {
    let unfolded = ics.replace("\r\n ", "").replace("\r\n\t", "");
    let vevent = &unfolded[unfolded.find("BEGIN:VEVENT").unwrap()..];
    let line = |prefix: &str| {
        vevent
            .lines()
            .find(|line| line.starts_with(prefix))
            .unwrap()
            .trim()
            .to_string()
    };
    format!("{}\n{}", line("DTSTART"), line("RRULE:"))
}

/// What a client expands from the invite, as UTC instants.
fn client_instances(
    zone: chrono_tz::Tz,
    start: NaiveDateTime,
    rule: &Recurrence,
) -> Vec<NaiveDateTime> {
    let ics = build(&series_invite(zone, start, rule.clone())).unwrap();
    let set: RRuleSet = emitted_rule(&ics).parse().unwrap();
    set.all(400)
        .dates
        .into_iter()
        .map(|instance| instance.with_timezone(&Utc).naive_utc())
        .collect()
}

fn rule(frequency: Frequency, count: u32) -> Recurrence {
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
        ..rule(frequency, count)
    }
}

#[test]
fn series_slots_equal_the_instances_a_client_expands() {
    let cases: Vec<(chrono_tz::Tz, NaiveDateTime, Recurrence)> = vec![
        // Every frequency across fall-back and spring-forward.
        (
            Chicago,
            wall(2026, 10, 20, 10, 0),
            rule(Frequency::Daily, 30),
        ),
        (
            Chicago,
            wall(2026, 10, 20, 10, 0),
            rule(Frequency::Weekly, 8),
        ),
        (
            Chicago,
            wall(2026, 10, 20, 10, 0),
            rule(Frequency::Biweekly, 6),
        ),
        (
            Chicago,
            wall(2026, 10, 20, 10, 0),
            rule(Frequency::Monthly, 7),
        ),
        (Chicago, wall(2027, 3, 2, 10, 0), rule(Frequency::Weekly, 6)),
        (London, wall(2026, 10, 13, 9, 0), rule(Frequency::Weekly, 6)),
        (London, wall(2027, 3, 16, 9, 0), rule(Frequency::Daily, 20)),
        (Sydney, wall(2027, 3, 23, 9, 0), rule(Frequency::Weekly, 6)),
        (Sydney, wall(2027, 9, 21, 9, 0), rule(Frequency::Monthly, 4)),
        (
            Kolkata,
            wall(2026, 10, 20, 18, 30),
            rule(Frequency::Weekly, 6),
        ),
        (
            chrono_tz::UTC,
            wall(2026, 10, 20, 15, 0),
            rule(Frequency::Weekly, 6),
        ),
        // The production series, `until`-terminated.
        (
            Chicago,
            wall(2026, 9, 10, 11, 0),
            Recurrence {
                until: Some(wall(2027, 1, 1, 5, 59)),
                count: None,
                ..on(&[Weekday::Thu], Frequency::Weekly, 1)
            },
        ),
        // Weekdays whose UTC day differs from the local one.
        (
            Chicago,
            wall(2026, 10, 22, 19, 0),
            on(&[Weekday::Thu, Weekday::Fri], Frequency::Weekly, 8),
        ),
        (
            Tokyo,
            wall(2026, 10, 19, 8, 0),
            on(&[Weekday::Mon, Weekday::Wed], Frequency::Biweekly, 6),
        ),
        // Multi-week steps with Sunday, which depend on WKST.
        (
            chrono_tz::UTC,
            wall(2027, 1, 4, 10, 0),
            on(&[Weekday::Mon, Weekday::Sun], Frequency::Biweekly, 6),
        ),
        (
            Chicago,
            wall(2027, 1, 4, 10, 0),
            on(&[Weekday::Mon, Weekday::Sun], Frequency::Biweekly, 6),
        ),
        // Every three weeks, and a monthly start on the 28th in the evening.
        (
            Chicago,
            wall(2026, 10, 20, 10, 0),
            Recurrence {
                interval: 3,
                ..rule(Frequency::Weekly, 6)
            },
        ),
        (
            Chicago,
            wall(2027, 1, 28, 19, 0),
            rule(Frequency::Monthly, 4),
        ),
        // A skipped and a repeated local time.
        (Chicago, wall(2027, 3, 12, 2, 30), rule(Frequency::Daily, 4)),
        (
            Chicago,
            wall(2026, 10, 30, 1, 30),
            rule(Frequency::Daily, 4),
        ),
    ];

    for (zone, local_start, recurrence) in cases {
        let start = at(zone, local_start);
        let slots = expand_recurrence(start, &recurrence, zone).unwrap();
        assert_eq!(
            slots,
            client_instances(zone, start, &recurrence),
            "{zone} from {local_start}: {recurrence:?}"
        );
    }
}

/// Stored rules that new series can no longer have still expand to what a client holds.
#[test]
fn stored_slots_equal_the_instances_a_client_expands() {
    let cases: Vec<(chrono_tz::Tz, NaiveDateTime, Recurrence)> = vec![
        (
            chrono_tz::UTC,
            wall(2026, 1, 31, 10, 0),
            rule(Frequency::Monthly, 6),
        ),
        (
            Chicago,
            wall(2026, 10, 30, 19, 0),
            rule(Frequency::Monthly, 6),
        ),
        (
            Chicago,
            wall(2026, 12, 31, 9, 0),
            Recurrence {
                interval: 2,
                ..rule(Frequency::Monthly, 3)
            },
        ),
    ];

    for (zone, local_start, recurrence) in cases {
        let start = at(zone, local_start);
        let slots = expand_published(start, &recurrence, zone).unwrap();
        assert_eq!(
            slots,
            client_instances(zone, start, &recurrence),
            "{zone} from {local_start}: {recurrence:?}"
        );
    }
}
