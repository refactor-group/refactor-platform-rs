//! The rendered model input and the short refs it uses.

use chrono::{NaiveDate, Utc, Weekday};

use super::*;
use crate::coaching_relationship_kind::{self, Kind};
use crate::transcript_export::LabeledSegment;
use crate::transcript_suggestion::{Existing, NextSession, People, Session, Timezones};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

fn segment(
    label: &str,
    role: Option<coaching_relationship_kind::Role>,
    start_ms: i32,
    text: &str,
) -> LabeledSegment {
    LabeledSegment {
        id: Id::new_v4(),
        transcription_id: Id::nil(),
        speaker_label: label.to_string(),
        speaker_user_id: None,
        speaker_role: role,
        text: text.to_string(),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().fixed_offset(),
    }
}

fn segments() -> Vec<LabeledSegment> {
    vec![
        segment(
            "Jim Hodapp",
            Some(coaching_relationship_kind::Role::Coach),
            5_000,
            "Good to see you.",
        ),
        segment("Priya Shah", None, 65_000, "I'll send the org chart."),
        segment(
            "Alex Rivera",
            Some(coaching_relationship_kind::Role::Coachee),
            4_515_000,
            "I'll send it\nby Friday.",
        ),
    ]
}

fn session(segments: &[LabeledSegment], next_session: Option<NaiveDate>) -> Session<'_> {
    Session {
        coaching_session_id: Id::new_v4(),
        people: People {
            coach_id: Id::new_v4(),
            coachee_id: Id::new_v4(),
        },
        coach_name: "Jim Hodapp".to_string(),
        coachee_name: "Alex Rivera".to_string(),
        anchor_timezone: chrono_tz::America::Chicago,
        timezones: Timezones {
            coach: chrono_tz::America::Chicago,
            coachee: chrono_tz::America::Chicago,
        },
        kind: Kind::Coaching,
        start: day(2026, 10, 7)
            .and_hms_opt(15, 0, 0)
            .expect("valid time")
            .and_utc()
            .fixed_offset(),
        session_date: day(2026, 10, 7),
        next_session: next_session.map(|date| NextSession {
            date,
            start: date
                .and_hms_opt(15, 0, 0)
                .expect("valid time")
                .and_utc()
                .fixed_offset(),
        }),
        week_end: Weekday::Fri,
        segments,
        existing: vec![Existing {
            id: Id::new_v4(),
            text: "Update the\n  onboarding doc".to_string(),
        }],
    }
}

#[test]
fn render_writes_the_session_existing_actions_and_transcript() {
    let segments = segments();
    let rendered = render(&session(&segments, None));
    let lines: Vec<&str> = rendered.lines().collect();

    let calendar_start = 6;
    let calendar_end = lines
        .iter()
        .position(|l| l.is_empty())
        .expect("blank line after the calendar");
    let calendar = &lines[calendar_start..calendar_end];
    let without_calendar = [&lines[..calendar_start], &lines[calendar_end..]]
        .concat()
        .join("\n");

    assert_eq!(
        without_calendar,
        "Coach: Jim Hodapp\n\
         Coachee: Alex Rivera\n\
         Session date: Wednesday 2026-10-07\n\
         Next session: none scheduled\n\
         Week ends on: Friday\n\
         Calendar (the 60 days after the session):\n\
         \n\
         Existing actions in this session:\n\
         [a1] Update the onboarding doc\n\
         \n\
         Transcript:\n\
         [s1 00:05] Coach: Good to see you.\n\
         [s2 01:05] Priya Shah: I'll send the org chart.\n\
         [s3 75:15] Coachee: I'll send it by Friday."
    );
    assert!(!rendered.ends_with('\n'));

    assert_eq!(
        calendar.first().copied(),
        Some("Wed 2026-10-07, Thu 2026-10-08, Fri 2026-10-09, Sat 2026-10-10, Sun 2026-10-11")
    );
    assert!(calendar
        .last()
        .expect("calendar lines")
        .ends_with("Sun 2026-12-06"));
    assert!(!rendered.contains("2026-12-07"));
    // The header plus nine Monday-Sunday weeks.
    assert_eq!(lines[calendar_start - 1..calendar_end].len(), 10);
}

#[test]
fn render_names_the_next_session() {
    let segments = segments();
    let rendered = render(&session(&segments, Some(day(2026, 10, 21))));
    assert!(rendered
        .lines()
        .any(|l| l == "Next session: Wednesday 2026-10-21"));
}

#[test]
fn render_without_existing_actions_says_none() {
    let segments = segments();
    let mut session = session(&segments, None);
    session.existing.clear();
    assert!(render(&session).contains("Existing actions in this session:\n(none)\n"));
}

#[test]
fn refs_map_short_refs_to_ids() {
    let segments = segments();
    let session = session(&segments, None);
    let refs = Refs::new(&session);

    assert_eq!(refs.segment("s1"), Some(segments[0].id));
    assert_eq!(refs.segment("s3"), Some(segments[2].id));
    for r in ["s0", "s4", "x1", "a1", "s", "S1", "", "s01", "s+1"] {
        assert_eq!(refs.segment(r), None, "{r}");
    }
    assert_eq!(refs.existing("a1"), Some(session.existing[0].id));
    assert_eq!(refs.existing("a2"), None);
    assert_eq!(
        refs.segment_text(&session, segments[1].id),
        Some("I'll send the org chart.")
    );
}

#[test]
fn a_colleague_line_is_named_by_its_label_like_an_unattributed_line() {
    let colleague = segment(
        "Sam Lee",
        Some(coaching_relationship_kind::Role::Colleague),
        5_000,
        "Let's pair on it.",
    );
    let unattributed = segment("Sam Lee", None, 5_000, "Let's pair on it.");
    let session = session(&[], None);

    assert_eq!(
        transcript_line(&session, 1, &colleague),
        "[s1 00:05] Sam Lee: Let's pair on it."
    );
    assert_eq!(
        transcript_line(&session, 1, &colleague),
        transcript_line(&session, 1, &unattributed)
    );
}

#[test]
fn a_colleague_render_names_both_people_and_labels_their_lines_with_those_names() {
    let mut segments = vec![
        segment(
            "Bob (he/him)",
            Some(coaching_relationship_kind::Role::Colleague),
            5_000,
            "I'll draft it.",
        ),
        segment("Guest 1", None, 9_000, "Sounds good."),
    ];
    let bob = Id::new_v4();
    segments[0].speaker_user_id = Some(bob);
    let session = Session {
        people: People {
            coach_id: Id::new_v4(),
            coachee_id: bob,
        },
        coach_name: "Alice Smith".to_string(),
        coachee_name: "Bob Lee".to_string(),
        kind: Kind::Colleague,
        ..session(&segments, None)
    };

    let rendered = render(&session);
    let lines: Vec<&str> = rendered.lines().collect();

    assert!(lines.contains(&"Colleagues: Alice Smith, Bob Lee"));
    assert!(!lines
        .iter()
        .any(|l| l.starts_with("Coach:") || l.starts_with("Coachee:")));
    assert!(lines.contains(&"[s1 00:05] Bob Lee: I'll draft it."));
    assert!(lines.contains(&"[s2 00:09] Guest 1: Sounds good."));
}

#[test]
fn distinct_names_fall_back_to_full_names_then_to_numbers() {
    let names = |preferred: [&str; 2], full: [&str; 2]| {
        distinct_names(preferred.map(str::to_string), full.map(str::to_string))
    };

    assert_eq!(
        names(["Alex", "Sam"], ["Alex Kim", "Sam Park"]),
        ["Alex", "Sam"]
    );
    assert_eq!(
        names(["Alex", "Alex"], ["Alex Kim", "Alex Park"]),
        ["Alex Kim", "Alex Park"]
    );
    assert_eq!(
        names(["Alex", "Alex"], ["Alex Kim", "Alex Kim"]),
        ["Alex (1)", "Alex (2)"]
    );
}
