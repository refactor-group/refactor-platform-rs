//! What each series email says, in every branch that changes its copy.

use super::*;
use crate::test_utils::email_composition::{
    bodies, copy, series_cancelled, series_rescheduled, series_rescheduled_from_session,
    series_scheduled, when,
};

#[test]
fn series_scheduled_copy() {
    insta::assert_snapshot!("series_scheduled_by_other", copy(&series_scheduled()));
    insta::assert_snapshot!(
        "series_scheduled_by_you",
        copy(&SeriesScheduled {
            by: Actor::You,
            ..series_scheduled()
        })
    );
}

#[test]
fn series_rescheduled_copy() {
    insta::assert_snapshot!("series_rescheduled_by_other", copy(&series_rescheduled()));
    insta::assert_snapshot!(
        "series_rescheduled_by_you",
        copy(&SeriesRescheduled {
            by: Actor::You,
            ..series_rescheduled()
        })
    );
    insta::assert_snapshot!(
        "series_rescheduled_from_session_by_other",
        copy(&series_rescheduled_from_session())
    );
    insta::assert_snapshot!(
        "series_rescheduled_from_session_by_you",
        copy(&SeriesRescheduled {
            by: Actor::You,
            ..series_rescheduled_from_session()
        })
    );
}

#[test]
fn series_cancelled_copy() {
    insta::assert_snapshot!("series_cancelled_by_other", copy(&series_cancelled()));
    insta::assert_snapshot!(
        "series_cancelled_by_you",
        copy(&SeriesCancelled {
            by: Actor::You,
            ..series_cancelled()
        })
    );
}

#[test]
fn a_whole_series_reschedule_shows_only_what_changed() {
    let start = || Some(when("Tuesday, March 3, 2026", "4:00 PM"));
    let cadence = || Some("Weekly on Tuesday".to_string());
    for (previous_start, previous_cadence, started, repeated) in [
        (start(), cadence(), true, true),
        (None, cadence(), false, true),
        (start(), None, true, false),
        (None, None, false, false),
    ] {
        let (html, text) = bodies(&SeriesRescheduled {
            scope: Scope::WholeSeries {
                previous_start,
                previous_cadence,
            },
            ..series_rescheduled()
        });

        assert_eq!(html.contains("Previously started:</strong>"), started);
        assert_eq!(text.contains("\nPreviously started: "), started);
        assert_eq!(html.contains("Previously repeated:</strong>"), repeated);
        assert_eq!(text.contains("\nPreviously repeated: "), repeated);
    }
}
