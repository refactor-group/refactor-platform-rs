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
    let start = || Some(when("Tuesday, March 3, 2026", "Tue, Mar 3", "4:00 PM"));
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

        assert_eq!(html.contains(r#"class="before""#), started || repeated);
        assert_eq!(text.contains("\nBefore:\n"), started || repeated);
        assert_eq!(html.contains("March 3, 2026 at 4:00 PM"), started);
        assert_eq!(text.contains("March 3, 2026 at 4:00 PM"), started);
        assert_eq!(html.contains("Weekly on Tuesday"), repeated);
        assert_eq!(text.contains("Weekly on Tuesday"), repeated);
    }
}
