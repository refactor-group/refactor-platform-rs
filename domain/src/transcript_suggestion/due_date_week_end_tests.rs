//! "This week" and the end of the week follow the day the coach's week ends on.

use chrono::{NaiveDate, Weekday};

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Anchors for a session on `session_date` with no next session.
fn week_ending(week_end: Weekday, session_date: NaiveDate) -> Anchors {
    Anchors {
        session_date,
        next_session: None,
        week_end,
    }
}

const WED: (i32, u32, u32) = (2026, 10, 7);

#[test]
fn this_week_means_the_end_of_the_week() {
    let anchors = week_ending(DEFAULT_WEEK_END, day(WED.0, WED.1, WED.2));
    for phrase in [
        "this week",
        "by this week",
        "end of the week",
        "by the end of this week",
    ] {
        assert_eq!(
            anchors.resolve(phrase),
            Phrase::Date(day(2026, 10, 9)),
            "{phrase}"
        );
    }
}

#[test]
fn a_week_ending_saturday_moves_both_ends_of_the_week() {
    let anchors = week_ending(Weekday::Sat, day(WED.0, WED.1, WED.2));
    assert_eq!(
        anchors.resolve("this week"),
        Phrase::Date(day(2026, 10, 10))
    );
    assert_eq!(
        anchors.resolve("end of the week"),
        Phrase::Date(day(2026, 10, 10))
    );
    assert_eq!(
        anchors.resolve("end of next week"),
        Phrase::Date(day(2026, 10, 17))
    );
}

#[test]
fn a_session_on_the_last_day_of_the_week_is_due_that_day() {
    let anchors = week_ending(DEFAULT_WEEK_END, day(2026, 10, 9));
    assert_eq!(anchors.resolve("this week"), Phrase::Date(day(2026, 10, 9)));
}

#[test]
fn next_week_stays_ambiguous_and_other_days_ignore_the_week_end() {
    let anchors = week_ending(Weekday::Sat, day(WED.0, WED.1, WED.2));
    assert_eq!(anchors.resolve("next week"), Phrase::Ambiguous);
    assert_eq!(anchors.resolve("by Friday"), Phrase::Date(day(2026, 10, 9)));
    assert_eq!(
        anchors.resolve("this weekend"),
        Phrase::Date(day(2026, 10, 11))
    );
}

#[test]
fn reconcile_uses_the_week_end_over_the_models_date() {
    let anchors = week_ending(Weekday::Sat, day(WED.0, WED.1, WED.2));
    assert_eq!(
        anchors.reconcile(Some(day(2026, 10, 9)), Some("this week")),
        Some(day(2026, 10, 10))
    );
}
