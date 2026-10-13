//! Phrases that put a deadline later on the session's own day.

use chrono::NaiveDate;

use super::*;

fn wed() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 7).expect("valid date")
}

#[test]
fn after_the_call_and_later_today_resolve_to_the_session_date() {
    for phrase in [
        "after our call",
        "After this call",
        "after the call",
        "later today",
        "by later today",
    ] {
        assert_eq!(
            resolve_phrase(phrase, wed(), None),
            Phrase::Date(wed()),
            "{phrase}"
        );
    }
}

#[test]
fn a_same_day_phrase_overrides_a_missing_model_date() {
    assert_eq!(
        reconcile(None, Some("after our call"), wed(), None),
        Some(wed())
    );
}
