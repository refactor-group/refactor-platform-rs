//! What each session email says, in every branch that changes its copy.

use super::*;
use crate::test_utils::email_composition::{
    bodies, copy, session_cancelled, session_reminder, session_rescheduled, session_scheduled,
};

#[test]
fn session_scheduled_copy() {
    insta::assert_snapshot!("session_scheduled_by_other", copy(&session_scheduled()));
    insta::assert_snapshot!(
        "session_scheduled_by_you",
        copy(&SessionScheduled {
            by: Actor::You,
            ..session_scheduled()
        })
    );
}

#[test]
fn session_rescheduled_copy() {
    insta::assert_snapshot!("session_rescheduled", copy(&session_rescheduled()));
}

#[test]
fn session_cancelled_copy() {
    insta::assert_snapshot!("session_cancelled_by_other", copy(&session_cancelled()));
    insta::assert_snapshot!(
        "session_cancelled_by_you",
        copy(&SessionCancelled {
            reason: CancelReason::Cancelled { by: Actor::You },
            ..session_cancelled()
        })
    );
    insta::assert_snapshot!(
        "session_cancelled_superseded",
        copy(&SessionCancelled {
            reason: CancelReason::Superseded,
            ..session_cancelled()
        })
    );
}

#[test]
fn session_reminder_copy() {
    insta::assert_snapshot!("session_reminder", copy(&session_reminder()));
}

#[test]
fn a_reschedule_shows_the_previous_time_only_when_the_start_moved() {
    for (previous, shown) in [(session_rescheduled().previous, true), (None, false)] {
        let (html, text) = bodies(&SessionRescheduled {
            previous,
            ..session_rescheduled()
        });

        assert_eq!(html.contains("Previous time:</strong>"), shown, "{html}");
        assert_eq!(text.contains("\nPrevious time: "), shown, "{text}");
    }
}

#[test]
fn a_reminder_shows_the_title_only_when_there_is_one() {
    for (title, shown) in [(session_reminder().title, true), (None, false)] {
        let (html, text) = bodies(&SessionReminder {
            title,
            ..session_reminder()
        });

        assert_eq!(html.contains("Session:</strong>"), shown, "{html}");
        assert_eq!(text.contains("\nSession: "), shown, "{text}");
    }
}
