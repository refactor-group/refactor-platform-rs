use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};

use super::*;
use crate::test_utils::email_outbox::outgoing_email;

fn at(hour: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 12, 1)
        .and_then(|day| day.and_hms_opt(hour, 0, 0))
        .expect("valid test time")
}

fn policy() -> RetryPolicy {
    RetryPolicy::default()
}

#[test]
fn backoff_doubles_from_the_base_up_to_the_cap() {
    let waits: Vec<_> = [1, 2, 3, 7, 8, i16::MAX]
        .into_iter()
        .map(|attempts| policy().backoff(attempts, None))
        .collect();

    assert_eq!(
        waits,
        [
            TimeDelta::seconds(30),
            TimeDelta::seconds(60),
            TimeDelta::seconds(120),
            TimeDelta::minutes(30),
            TimeDelta::minutes(30),
            TimeDelta::minutes(30),
        ]
    );
}

#[test]
fn backoff_waits_for_retry_after_only_when_it_is_longer() {
    assert_eq!(
        policy().backoff(1, Some(Duration::from_secs(90))),
        TimeDelta::seconds(90)
    );
    assert_eq!(
        policy().backoff(3, Some(Duration::from_secs(5))),
        TimeDelta::seconds(120)
    );
    assert_eq!(
        policy().backoff(1, Some(Duration::from_secs(u64::MAX))),
        policy().max_age,
        "a wait past the cutoff is clamped to it"
    );
}

#[test]
fn a_row_expires_at_the_max_age() {
    let created = at(0);

    assert!(!policy().is_expired(
        created,
        created + TimeDelta::hours(20) - TimeDelta::seconds(1)
    ));
    assert!(policy().is_expired(created, created + TimeDelta::hours(20)));
}

#[test]
fn settle_maps_each_receipt_to_the_rows_next_state() {
    let now = at(12);

    assert_eq!(
        settle(&Receipt::Accepted, 1, now, &policy()),
        Transition::Sent
    );
    assert_eq!(
        settle(
            &Receipt::Retry {
                after: None,
                error: "503".into()
            },
            2,
            now,
            &policy()
        ),
        Transition::Retry {
            not_before: now + TimeDelta::seconds(60),
            error: "503".into()
        }
    );
    assert_eq!(
        settle(&Receipt::Rejected("422".into()), 1, now, &policy()),
        Transition::Fail("422".into())
    );
}

#[test]
fn keys_and_subjects_have_stable_forms() {
    let id = Id::nil();

    assert_eq!(
        idempotency_key(id),
        "email-outbox/00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        Subject::CoachingRelationship(id).key(),
        "coaching_relationship/00000000-0000-0000-0000-000000000000"
    );
    assert_eq!(
        Subject::User(id).key(),
        "user/00000000-0000-0000-0000-000000000000"
    );
}

#[test]
fn an_outgoing_emails_debug_shows_addresses_but_not_its_contents() {
    let email = OutgoingEmail {
        attachments: vec![Attachment {
            filename: "invite.ics".to_string(),
            content_type: "text/calendar".to_string(),
            content: "c2VjcmV0".to_string(),
        }],
        ..outgoing_email("Reset your password")
    };

    let shown = format!("{email:?}");

    assert!(
        shown.contains(
            "from sender@example.com to recipient@example.com, replies to hello@example.com"
        ),
        "{shown}"
    );
    assert!(shown.contains("invite.ics"), "{shown}");
    assert!(!shown.contains("Reset your password"), "{shown}");
    assert!(!shown.contains("c2VjcmV0"), "{shown}");
}
