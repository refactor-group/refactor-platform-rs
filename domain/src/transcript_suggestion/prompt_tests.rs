//! The rendered model input and the short refs it uses.

use chrono::{NaiveDate, Utc, Weekday};

use super::*;
use crate::transcript_export::{LabeledSegment, SpeakerRole};
use crate::transcript_suggestion::{Existing, NextSession, People, Session};

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

fn segment(label: &str, role: Option<SpeakerRole>, start_ms: i32, text: &str) -> LabeledSegment {
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
            Some(SpeakerRole::Coach),
            5_000,
            "Good to see you.",
        ),
        segment("Priya Shah", None, 65_000, "I'll send the org chart."),
        segment(
            "Alex Rivera",
            Some(SpeakerRole::Coachee),
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
        coach_timezone: chrono_tz::America::Chicago,
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
