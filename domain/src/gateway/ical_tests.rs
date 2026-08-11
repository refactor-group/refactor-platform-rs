use super::*;
use crate::coaching_session::{Frequency, Recurrence};
use chrono::{NaiveDate, NaiveDateTime, Weekday};
use chrono_tz::America::New_York;
use entity::users::Model;
use sea_orm::prelude::DateTimeWithTimeZone;

fn ts() -> DateTimeWithTimeZone {
    DateTimeWithTimeZone::parse_from_rfc3339("2026-01-01T00:00:00Z").unwrap()
}

fn dt(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(y, m, d)
        .unwrap()
        .and_hms_opt(h, min, 0)
        .unwrap()
}

fn user(email: &str, first: &str, last: &str, display: Option<&str>) -> Model {
    Model {
        id: sea_orm::prelude::Uuid::nil(),
        email: email.into(),
        first_name: first.into(),
        last_name: last.into(),
        display_name: display.map(Into::into),
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".into(),
        default_coaching_session_duration_minutes: 60,
        role: entity::roles::Role::User,
        roles: vec![],
        invite_status: None,
        created_at: ts(),
        updated_at: ts(),
    }
}

fn invite<'a>(
    coach: &'a Model,
    coachee: &'a Model,
    anchor_tz: chrono_tz::Tz,
    recurrence: Option<Recurrence>,
) -> IcsInvite<'a> {
    IcsInvite {
        uid: "session-1@myrefactor.com".into(),
        sequence: 0,
        method: Method::Request,
        status: EventStatus::Confirmed,
        summary: "Coaching Session: Acme".into(),
        description: "desc".into(),
        anchor_tz,
        dtstamp: dt(2026, 6, 14, 12, 0),
        start: dt(2026, 9, 15, 19, 0),
        duration_minutes: 60,
        organizer: coach,
        attendee: coachee,
        location_url: None,
        recurrence,
        recurrence_id: None,
        sent_by: None,
    }
}

/// `SENT-BY` marks an agent sending on the organizer's behalf. Without it, a
/// recipient sees the From address disagree with `ORGANIZER` and may refuse to
/// apply the update. The value is quoted because a cal-address contains a colon.
#[test]
fn organizer_carries_sent_by_when_an_agent_sends() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let mut inv = invite(&coach, &coachee, New_York, None);
    inv.sent_by = Some("hello@mail.myrefactor.com");
    let out = build(&inv).unwrap().replace("\r\n ", "");

    assert!(
        out.contains("SENT-BY=\"mailto:hello@mail.myrefactor.com\""),
        "expected a quoted SENT-BY cal-address: {out}"
    );
    assert!(
        out.contains("mailto:coach@example.com"),
        "organizer must stay the coach"
    );
}

/// Omitting `sent_by` must leave the property exactly as before.
#[test]
fn organizer_omits_sent_by_when_absent() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let out = build(&invite(&coach, &coachee, New_York, None)).unwrap();
    assert!(!out.contains("SENT-BY"));
}

#[test]
fn single_request_known_zone() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let out = build(&invite(&coach, &coachee, New_York, None)).unwrap();

    assert!(out.contains("BEGIN:VTIMEZONE"));
    assert!(out.contains("TZID:America/New_York"));
    assert!(out.contains("DTSTART;TZID=America/New_York:20260915T150000"));
    assert!(out.contains("DTEND;TZID=America/New_York:"));
    assert!(out.contains("METHOD:REQUEST"));
    assert!(out.contains("STATUS:CONFIRMED"));
    assert!(out.contains("SEQUENCE:0"));
    assert!(out.contains("UID:session-1@myrefactor.com"));

    assert_eq!(out.matches("BEGIN:VTIMEZONE").count(), 1);
    assert!(out.find("BEGIN:VTIMEZONE").unwrap() < out.find("BEGIN:VEVENT").unwrap());
}

#[test]
fn utc_anchor_falls_back_to_z() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let out = build(&invite(&coach, &coachee, chrono_tz::UTC, None)).unwrap();

    assert!(!out.contains("BEGIN:VTIMEZONE"));
    assert!(!out.contains("TZID="));
    assert!(out.contains("DTSTART:20260915T190000Z"));
}

#[test]
fn recurring_biweekly_rrule() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let rule = Recurrence {
        frequency: Frequency::Biweekly,
        interval: 1,
        by_weekdays: None,
        count: None,
        until: None,
    };
    let out = build(&invite(&coach, &coachee, New_York, Some(rule))).unwrap();
    assert!(out.contains("RRULE:FREQ=WEEKLY;INTERVAL=2"));
}

#[test]
fn recurring_weekly_byday_count() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let rule = Recurrence {
        frequency: Frequency::Weekly,
        interval: 1,
        by_weekdays: Some(vec![Weekday::Mon, Weekday::Wed]),
        count: Some(8),
        until: None,
    };
    let out = build(&invite(&coach, &coachee, New_York, Some(rule))).unwrap();
    assert!(out.contains("FREQ=WEEKLY"));
    assert!(out.contains("BYDAY=MO,WE"));
    assert!(out.contains("COUNT=8"));
}

#[test]
fn cancel_shape() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let mut inv = invite(&coach, &coachee, New_York, None);
    inv.method = Method::Cancel;
    inv.status = EventStatus::Cancelled;
    let out = build(&inv).unwrap();
    assert!(out.contains("METHOD:CANCEL"));
    assert!(out.contains("STATUS:CANCELLED"));
}

/// A mismatched DTSTART/RECURRENCE-ID form is the classic reason clients fail to match
/// an override, so both must use the same zoned or `Z` representation.
#[test]
fn recurrence_id_matches_dtstart_timezone_form() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);

    let mut zoned = invite(&coach, &coachee, New_York, None);
    zoned.recurrence_id = Some(dt(2026, 9, 8, 19, 0));
    let out = build(&zoned).unwrap();
    assert!(out.contains("DTSTART;TZID=America/New_York:20260915T150000"));
    assert!(out.contains("RECURRENCE-ID;TZID=America/New_York:20260908T150000"));

    let mut utc = invite(&coach, &coachee, chrono_tz::UTC, None);
    utc.recurrence_id = Some(dt(2026, 9, 8, 19, 0));
    let out = build(&utc).unwrap();
    assert!(out.contains("DTSTART:20260915T190000Z"));
    assert!(out.contains("RECURRENCE-ID:20260908T190000Z"));
    assert!(!out.contains("TZID="));
}

/// An override instance addresses a single occurrence, so any RRULE is dropped.
#[test]
fn override_instance_never_carries_an_rrule() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let rule = Recurrence {
        frequency: Frequency::Weekly,
        interval: 1,
        by_weekdays: None,
        count: None,
        until: None,
    };
    let mut inv = invite(&coach, &coachee, New_York, Some(rule));
    inv.recurrence_id = Some(dt(2026, 9, 8, 19, 0));
    let out = build(&inv).unwrap();

    // Slice the VEVENT: the spliced VTIMEZONE carries its own daylight-saving RRULEs.
    let start = out.find("BEGIN:VEVENT").unwrap();
    let end = out.find("END:VEVENT").unwrap();
    assert!(!out[start..end].contains("RRULE"));
    assert!(out.contains("RECURRENCE-ID;TZID=America/New_York:20260908T150000"));
}

/// RFC 5545 line unfolding: remove the CRLF followed by a leading space/tab.
fn unfold(s: &str) -> String {
    s.replace("\r\n ", "").replace("\r\n\t", "")
}

#[test]
fn attendee_organizer_lines() {
    let coach = user("coach@example.com", "Coach", "Casey", Some("Coach Casey"));
    let coachee = user("coachee@example.com", "Coachee", "Quinn", None);
    let raw = build(&invite(&coach, &coachee, New_York, None)).unwrap();
    let out = unfold(&raw);

    assert!(out.contains("ORGANIZER;CN="));
    assert!(out.contains("CN=Coach Casey"));
    assert!(out.contains("mailto:coach@example.com"));
    assert!(out.contains("CN=Coachee Quinn"));
    assert!(out.contains("mailto:coachee@example.com"));
    assert!(out.contains("ATTENDEE"));
    assert!(out.contains("RSVP=TRUE"));
}

#[test]
fn description_full_single() {
    let topics = vec!["Roadmap".to_string(), "Hiring".to_string()];
    let goals = vec!["Ship v2".to_string()];
    let actions = vec![
        OpenAction {
            body: "Draft doc".into(),
            due_by: Some(dt(2026, 9, 15, 19, 0)),
        },
        OpenAction {
            body: "Email team".into(),
            due_by: None,
        },
    ];
    let parts = DescriptionParts {
        session_url: "https://app.example/sessions/1".into(),
        title: Some("Q3 Planning"),
        topics: &topics,
        goal_titles: &goals,
        open_actions: &actions,
        anchor_tz: New_York,
    };
    let out = compose_description(&parts);

    assert!(out.contains("Q3 Planning"));
    assert!(out.contains("View this session: https://app.example/sessions/1"));
    assert!(out.contains("Topics to discuss:"));
    assert!(out.contains("- Roadmap"));
    assert!(out.contains("- Hiring"));
    assert!(out.contains("Goals you're working toward:"));
    assert!(out.contains("- Ship v2"));
    assert!(out.contains("Actions due for this session:"));
    assert!(out.contains("- Draft doc (due Sep 15, 2026)"));
    assert!(out.contains("- Email team (no due date)"));
}

#[test]
fn description_minimal_series_shape() {
    let goals = vec!["Ship v2".to_string()];
    let parts = DescriptionParts {
        session_url: "https://app.example/sessions/1".into(),
        title: None,
        topics: &[],
        goal_titles: &goals,
        open_actions: &[],
        anchor_tz: New_York,
    };
    let out = compose_description(&parts);

    assert!(out.contains("View this session:"));
    assert!(out.contains("- Ship v2"));
    assert!(!out.contains("Topics to discuss:"));
    assert!(!out.contains("Actions due for this session:"));
    assert!(!out.contains("Q3 Planning"));
}
