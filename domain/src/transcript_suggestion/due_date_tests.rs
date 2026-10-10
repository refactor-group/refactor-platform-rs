//! Spoken deadlines resolved against the session date, and reconciled with the model's date.

use chrono::NaiveDate;

use super::*;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Wednesday.
fn wed() -> NaiveDate {
    day(2026, 10, 7)
}

/// Friday.
fn fri() -> NaiveDate {
    day(2026, 10, 9)
}

fn next_session() -> Option<NaiveDate> {
    Some(day(2026, 10, 21))
}

fn on_wed(phrase: &str) -> Phrase {
    resolve_phrase(phrase, wed(), next_session())
}

#[test]
fn same_day_phrases_resolve_to_the_session_date() {
    for phrase in [
        "today",
        "tonight",
        "EOD",
        "by EOD",
        "end of day",
        "by the end of the day",
    ] {
        assert_eq!(on_wed(phrase), Phrase::Date(wed()), "{phrase}");
    }
}

#[test]
fn tomorrow_phrases_resolve_to_the_next_day() {
    for phrase in [
        "tomorrow",
        "by tomorrow",
        "Tomorrow.",
        "EOD tomorrow",
        "by EOD tomorrow",
        "end of day tomorrow",
        "tomorrow morning",
        "tomorrow night",
    ] {
        assert_eq!(on_wed(phrase), Phrase::Date(day(2026, 10, 8)), "{phrase}");
    }
}

#[test]
fn a_weekday_is_the_first_one_after_the_session() {
    assert_eq!(on_wed("Friday"), Phrase::Date(fri()));
    assert_eq!(on_wed("by Friday"), Phrase::Date(fri()));
    assert_eq!(on_wed("this Friday"), Phrase::Date(fri()));
    assert_eq!(on_wed("on Friday"), Phrase::Date(fri()));
    assert_eq!(on_wed("by Friday,"), Phrase::Date(fri()));
    assert_eq!(on_wed("Monday"), Phrase::Date(day(2026, 10, 12)));
    assert_eq!(on_wed("by monday"), Phrase::Date(day(2026, 10, 12)));
    // The session's own weekday means a week out, never the same day.
    assert_eq!(on_wed("Wednesday"), Phrase::Date(day(2026, 10, 14)));
}

#[test]
fn next_weekday_still_ahead_in_the_same_week_is_ambiguous() {
    // On a Wednesday, Friday and Saturday are still to come this week.
    assert_eq!(on_wed("next Friday"), Phrase::Ambiguous);
    assert_eq!(on_wed("by next Friday"), Phrase::Ambiguous);
    assert_eq!(on_wed("next Saturday"), Phrase::Ambiguous);
}

#[test]
fn next_weekday_already_past_this_week_is_the_coming_one() {
    // On a Wednesday, this week's Monday and Tuesday are gone, so "next" can only mean the coming one.
    assert_eq!(on_wed("next Monday"), Phrase::Date(day(2026, 10, 12)));
    assert_eq!(on_wed("next Tuesday"), Phrase::Date(day(2026, 10, 13)));
    // On a Friday, Tuesday has passed this week.
    assert_eq!(
        resolve_phrase("next Tuesday", fri(), next_session()),
        Phrase::Date(day(2026, 10, 13))
    );
}

#[test]
fn in_n_days_or_weeks_counts_from_the_session() {
    assert_eq!(on_wed("in two weeks"), Phrase::Date(day(2026, 10, 21)));
    assert_eq!(on_wed("in 2 weeks"), Phrase::Date(day(2026, 10, 21)));
    assert_eq!(on_wed("in a week"), Phrase::Date(day(2026, 10, 14)));
    assert_eq!(on_wed("in one week"), Phrase::Date(day(2026, 10, 14)));
    assert_eq!(on_wed("in three days"), Phrase::Date(day(2026, 10, 10)));
    assert_eq!(on_wed("in 10 days"), Phrase::Date(day(2026, 10, 17)));
}

#[test]
fn n_weeks_from_a_weekday_counts_from_that_weekday() {
    assert_eq!(
        on_wed("two weeks from this Wednesday"),
        Phrase::Date(day(2026, 10, 21))
    );
    assert_eq!(
        on_wed("a week from Friday"),
        Phrase::Date(day(2026, 10, 16))
    );
    assert_eq!(
        on_wed("two weeks from today"),
        Phrase::Date(day(2026, 10, 21))
    );
    assert_eq!(on_wed("a week from now"), Phrase::Date(day(2026, 10, 14)));
}

#[test]
fn end_of_week_weekend_and_month() {
    assert_eq!(on_wed("by the end of the week"), Phrase::Date(fri()));
    assert_eq!(on_wed("end of this week"), Phrase::Date(fri()));
    assert_eq!(on_wed("end of week"), Phrase::Date(fri()));
    assert_eq!(on_wed("end of next week"), Phrase::Date(day(2026, 10, 16)));
    assert_eq!(on_wed("this weekend"), Phrase::Date(day(2026, 10, 11)));
    assert_eq!(on_wed("over the weekend"), Phrase::Date(day(2026, 10, 11)));
    assert_eq!(
        on_wed("by the end of the month"),
        Phrase::Date(day(2026, 10, 31))
    );
    assert_eq!(on_wed("end of this month"), Phrase::Date(day(2026, 10, 31)));
    assert_eq!(
        resolve_phrase("end of the month", day(2027, 2, 3), None),
        Phrase::Date(day(2027, 2, 28))
    );
}

#[test]
fn next_session_phrases_use_the_next_session_date() {
    for phrase in [
        "before our next session",
        "by our next session",
        "before next session",
        "next session",
        "by next time",
        "before next time",
        "next time",
        "before we meet again",
        "by the time we meet again",
        "before our next meeting",
    ] {
        assert_eq!(on_wed(phrase), Phrase::Date(day(2026, 10, 21)), "{phrase}");
    }
}

#[test]
fn next_session_phrases_without_a_next_session_are_ambiguous() {
    assert_eq!(
        resolve_phrase("before our next session", wed(), None),
        Phrase::Ambiguous
    );
    assert_eq!(resolve_phrase("next time", wed(), None), Phrase::Ambiguous);
}

#[test]
fn explicit_dates_resolve_in_the_session_year_or_the_next() {
    assert_eq!(on_wed("October 20th"), Phrase::Date(day(2026, 10, 20)));
    assert_eq!(on_wed("by October 20"), Phrase::Date(day(2026, 10, 20)));
    assert_eq!(on_wed("Oct 20"), Phrase::Date(day(2026, 10, 20)));
    assert_eq!(
        on_wed("on the 20th of October"),
        Phrase::Date(day(2026, 10, 20))
    );
    assert_eq!(on_wed("10/20"), Phrase::Date(day(2026, 10, 20)));
    assert_eq!(on_wed("November 3rd"), Phrase::Date(day(2026, 11, 3)));
    // Already past this year, so next year's.
    assert_eq!(on_wed("January 15"), Phrase::Date(day(2027, 1, 15)));
    assert_eq!(on_wed("1/15"), Phrase::Date(day(2027, 1, 15)));
}

#[test]
fn vague_timing_is_ambiguous() {
    for phrase in [
        "soon",
        "ASAP",
        "as soon as possible",
        "at some point",
        "at some point this year",
        "sometime",
        "eventually",
        "later",
        "next week",
        "by next week",
        "next month",
        "this year",
        "later this year",
    ] {
        assert_eq!(on_wed(phrase), Phrase::Ambiguous, "{phrase}");
    }
}

#[test]
fn unknown_phrases_are_unrecognized() {
    for phrase in [
        "the Monday after Thanksgiving",
        "before the board meeting",
        "when the reorg lands",
        "February 30",
    ] {
        assert_eq!(on_wed(phrase), Phrase::Unrecognized, "{phrase}");
    }
}

// --- reconcile: the model's date checked against the phrase ---------------------------------

fn reconcile_on_wed(model: Option<NaiveDate>, phrase: Option<&str>) -> Option<NaiveDate> {
    reconcile(model, phrase, wed(), next_session())
}

#[test]
fn a_recognized_phrase_wins_over_a_wrong_model_date() {
    // Off by one, the slip seen in the spike.
    assert_eq!(
        reconcile_on_wed(Some(day(2026, 10, 10)), Some("by Friday")),
        Some(fri())
    );
    assert_eq!(
        reconcile_on_wed(Some(fri()), Some("by Friday")),
        Some(fri())
    );
    // The phrase resolves even when the model gave no date.
    assert_eq!(
        reconcile_on_wed(None, Some("tomorrow")),
        Some(day(2026, 10, 8))
    );
}

#[test]
fn an_ambiguous_phrase_clears_any_model_date() {
    assert_eq!(
        reconcile_on_wed(Some(day(2026, 10, 16)), Some("by next Friday")),
        None
    );
    assert_eq!(reconcile_on_wed(Some(day(2026, 10, 9)), Some("soon")), None);
    assert_eq!(
        reconcile(Some(day(2026, 10, 21)), Some("next time"), wed(), None),
        None
    );
}

#[test]
fn an_unrecognized_phrase_keeps_the_model_date_within_bounds() {
    assert_eq!(
        reconcile_on_wed(
            Some(day(2026, 11, 30)),
            Some("the Monday after Thanksgiving")
        ),
        Some(day(2026, 11, 30))
    );
    // Before the session: cleared.
    assert_eq!(
        reconcile_on_wed(Some(day(2026, 10, 6)), Some("before the board meeting")),
        None
    );
    // More than a year out: cleared.
    assert_eq!(
        reconcile_on_wed(Some(day(2027, 10, 8)), Some("before the board meeting")),
        None
    );
}

#[test]
fn a_model_date_without_a_phrase_is_kept_only_within_bounds() {
    assert_eq!(reconcile_on_wed(Some(fri()), None), Some(fri()));
    assert_eq!(reconcile_on_wed(Some(wed()), None), Some(wed()));
    assert_eq!(
        reconcile_on_wed(Some(day(2027, 10, 7)), None),
        Some(day(2027, 10, 7))
    );
    assert_eq!(reconcile_on_wed(Some(day(2026, 10, 6)), None), None);
    assert_eq!(reconcile_on_wed(Some(day(2027, 10, 8)), None), None);
    assert_eq!(reconcile_on_wed(None, None), None);
}
