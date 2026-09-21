//! Frozen acceptance tests for transcript speaker resolution and plain-text rendering.
//! Written by the overseer before implementation; read-only during the build.

use super::*;
use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
use crate::Id;
use chrono::Utc;

fn user(first: &str, last: &str, display: Option<&str>) -> users::Model {
    let now = Utc::now();
    users::Model {
        id: Id::new_v4(),
        email: format!("{}@test.com", first.to_lowercase()),
        first_name: first.to_owned(),
        last_name: last.to_owned(),
        display_name: display.map(str::to_owned),
        password: None,
        github_username: None,
        github_profile_url: None,
        timezone: "UTC".to_string(),
        default_coaching_session_duration_minutes: crate::duration::Duration::default_minutes(),
        created_at: now.into(),
        updated_at: now.into(),
        roles: vec![],
        invite_status: None,
    }
}

fn coach() -> users::Model {
    user("Jim", "Hodapp", Some("Jim H"))
}

fn coachee() -> users::Model {
    user("Caleb", "Bourg", None)
}

fn segment_with_id(id: Id, label: &str, text: &str, start_ms: i32) -> Segment {
    Segment {
        id,
        transcription_id: Id::nil(),
        speaker_label: label.to_owned(),
        text: text.to_owned(),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().into(),
    }
}

fn segment(label: &str, text: &str, start_ms: i32) -> Segment {
    segment_with_id(Id::new_v4(), label, text, start_ms)
}

fn speaker(label: &str, role: Option<SpeakerRole>) -> Speaker {
    Speaker {
        label: label.to_owned(),
        role,
    }
}

fn date() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 9, 21).expect("valid date")
}

fn labels(speakers: &[Speaker]) -> Vec<&str> {
    speakers.iter().map(|s| s.label.as_str()).collect()
}

fn roles(speakers: &[Speaker]) -> Vec<Option<SpeakerRole>> {
    speakers.iter().map(|s| s.role).collect()
}

// ---- resolve_speakers ----

#[test]
fn resolver_matches_display_name_full_name_and_first_name() {
    let segments = [
        segment("Jim H", "a", 0),
        segment("Caleb Bourg", "b", 1000),
        segment("Caleb", "c", 2000),
    ];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(labels(&speakers), ["Jim H", "Caleb Bourg", "Caleb"]);
    assert_eq!(
        roles(&speakers),
        [Some(SpeakerRole::Coach), Some(SpeakerRole::Coachee), None]
    );
}

#[test]
fn resolver_falls_back_to_first_name_alone() {
    let segments = [segment("caleb", "a", 0), segment("Jim H", "b", 1000)];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(
        roles(&speakers),
        [Some(SpeakerRole::Coachee), Some(SpeakerRole::Coach)]
    );
}

#[test]
fn resolver_prefers_full_name_over_first_name_when_both_present() {
    let segments = [segment("Caleb", "a", 0), segment("Caleb Bourg", "b", 1000)];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(labels(&speakers), ["Caleb", "Caleb Bourg"]);
    assert_eq!(roles(&speakers), [None, Some(SpeakerRole::Coachee)]);
}

#[test]
fn resolver_is_case_and_whitespace_insensitive() {
    let segments = [
        segment("  jim h ", "a", 0),
        segment("CALEB   BOURG", "b", 1000),
    ];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(labels(&speakers), ["  jim h ", "CALEB   BOURG"]);
    assert_eq!(
        roles(&speakers),
        [Some(SpeakerRole::Coach), Some(SpeakerRole::Coachee)]
    );
}

#[test]
fn resolver_leaves_unknown_and_guests_unresolved() {
    let segments = [
        segment("Unknown", "a", 0),
        segment("Guest", "b", 1000),
        segment("Jim H", "c", 2000),
    ];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(roles(&speakers), [None, None, Some(SpeakerRole::Coach)]);
}

#[test]
fn resolver_lists_each_label_once_in_first_appearance_order() {
    let segments = [
        segment("Caleb Bourg", "a", 0),
        segment("Jim H", "b", 1000),
        segment("Caleb Bourg", "c", 2000),
        segment("Jim H", "d", 3000),
    ];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(labels(&speakers), ["Caleb Bourg", "Jim H"]);
}

#[test]
fn resolver_lets_each_user_claim_at_most_one_label() {
    let segments = [segment("Jim H", "a", 0), segment("Jim Hodapp", "b", 1000)];
    let speakers = resolve_speakers(&coach(), &coachee(), &segments);
    assert_eq!(roles(&speakers), [Some(SpeakerRole::Coach), None]);
}

#[test]
fn resolver_gives_a_shared_label_to_the_coach_first() {
    let sam_coach = user("Sam", "Coach", None);
    let sam_coachee = user("Sam", "Coachee", None);
    let segments = [segment("Sam", "a", 0)];
    let speakers = resolve_speakers(&sam_coach, &sam_coachee, &segments);
    assert_eq!(roles(&speakers), [Some(SpeakerRole::Coach)]);
}

#[test]
fn resolver_returns_empty_for_no_segments() {
    assert!(resolve_speakers(&coach(), &coachee(), &[]).is_empty());
}

// ---- format_timestamp ----

#[test]
fn timestamp_boundaries() {
    assert_eq!(format_timestamp(0), "0:00");
    assert_eq!(format_timestamp(59_000), "0:59");
    assert_eq!(format_timestamp(60_000), "1:00");
    assert_eq!(format_timestamp(3_599_000), "59:59");
    assert_eq!(format_timestamp(3_600_000), "1:00:00");
    assert_eq!(format_timestamp(3_735_000), "1:02:15");
}

#[test]
fn timestamp_truncates_sub_second_remainder() {
    assert_eq!(format_timestamp(999), "0:00");
    assert_eq!(format_timestamp(61_999), "1:01");
}

// ---- render_plain_text ----

fn three_speakers() -> Vec<Speaker> {
    vec![
        speaker("Jim H", Some(SpeakerRole::Coach)),
        speaker("Caleb Bourg", Some(SpeakerRole::Coachee)),
        speaker("Guest", None),
    ]
}

fn three_speaker_segments() -> Vec<Segment> {
    vec![
        segment("Jim H", "Good morning.", 0),
        segment("Caleb Bourg", "Morning.", 4000),
        segment("Guest", "Hi both.", 9000),
        segment("Jim H", "Wrapping up.", 3_735_000),
    ]
}

#[test]
fn render_unfiltered_pins_the_exact_body_and_filename() {
    let rendered = render_plain_text(date(), &three_speakers(), &three_speaker_segments(), &[])
        .expect("renders");
    assert_eq!(
        rendered.body,
        "Coaching session transcript\n\
         Date: 2026-09-21\n\
         Speakers: Jim H, Caleb Bourg, Guest\n\
         \n\
         [0:00] Jim H: Good morning.\n\
         [0:04] Caleb Bourg: Morning.\n\
         [0:09] Guest: Hi both.\n\
         [1:02:15] Jim H: Wrapping up.\n"
    );
    assert_eq!(rendered.filename, "transcript-2026-09-21.txt");
}

#[test]
fn render_filtered_to_coach_keeps_only_coach_lines() {
    let rendered = render_plain_text(
        date(),
        &three_speakers(),
        &three_speaker_segments(),
        &[SpeakerRole::Coach],
    )
    .expect("renders");
    assert_eq!(
        rendered.body,
        "Coaching session transcript\n\
         Date: 2026-09-21\n\
         Speakers: Jim H\n\
         \n\
         [0:00] Jim H: Good morning.\n\
         [1:02:15] Jim H: Wrapping up.\n"
    );
    assert_eq!(rendered.filename, "transcript-2026-09-21-filtered.txt");
}

#[test]
fn render_filtered_to_both_roles_drops_guests_and_keeps_appearance_order() {
    let rendered = render_plain_text(
        date(),
        &three_speakers(),
        &three_speaker_segments(),
        &[SpeakerRole::Coachee, SpeakerRole::Coach],
    )
    .expect("renders");
    assert!(rendered.body.contains("Speakers: Jim H, Caleb Bourg\n"));
    assert!(!rendered.body.contains("Guest"));
    assert_eq!(rendered.filename, "transcript-2026-09-21-filtered.txt");
}

#[test]
fn render_header_lists_only_labels_that_survive_the_filter() {
    let segments = vec![segment("Caleb Bourg", "Morning.", 0)];
    let rendered = render_plain_text(date(), &three_speakers(), &segments, &[SpeakerRole::Coach])
        .expect("renders");
    assert!(rendered.body.contains("Speakers: Jim H\n"));
    assert!(!rendered.body.contains("Caleb Bourg"));
}

#[test]
fn render_skips_whitespace_only_segments() {
    let segments = vec![
        segment("Jim H", "Hello.", 0),
        segment("Jim H", "   ", 1000),
        segment("Jim H", "\n\t", 2000),
        segment("Caleb Bourg", "Hi.", 3000),
    ];
    let rendered = render_plain_text(date(), &three_speakers(), &segments, &[]).expect("renders");
    assert_eq!(
        rendered.body.lines().skip(4).collect::<Vec<_>>(),
        ["[0:00] Jim H: Hello.", "[0:03] Caleb Bourg: Hi."]
    );
}

#[test]
fn render_trims_surrounding_whitespace_from_text() {
    let segments = vec![segment("Jim H", "  Hello there.  ", 0)];
    let rendered = render_plain_text(date(), &three_speakers(), &segments, &[]).expect("renders");
    assert!(rendered.body.ends_with("[0:00] Jim H: Hello there.\n"));
}

#[test]
fn render_sorts_by_start_then_id() {
    let low = Id::from_u128(1);
    let high = Id::from_u128(2);
    let segments = vec![
        segment_with_id(high, "Jim H", "second", 5000),
        segment_with_id(low, "Jim H", "first", 5000),
        segment("Caleb Bourg", "zeroth", 0),
    ];
    let rendered = render_plain_text(date(), &three_speakers(), &segments, &[]).expect("renders");
    assert_eq!(
        rendered.body.lines().skip(4).collect::<Vec<_>>(),
        [
            "[0:00] Caleb Bourg: zeroth",
            "[0:05] Jim H: first",
            "[0:05] Jim H: second"
        ]
    );
}

#[test]
fn render_with_no_segments_yields_header_only() {
    let rendered = render_plain_text(date(), &[], &[], &[]).expect("renders");
    assert_eq!(
        rendered.body,
        "Coaching session transcript\nDate: 2026-09-21\nSpeakers: \n\n"
    );
}

#[test]
fn render_fails_when_a_requested_role_has_no_label() {
    let speakers = vec![
        speaker("Jim H", Some(SpeakerRole::Coach)),
        speaker("Nobody", None),
    ];
    let err = render_plain_text(
        date(),
        &speakers,
        &three_speaker_segments(),
        &[SpeakerRole::Coach, SpeakerRole::Coachee],
    )
    .expect_err("coachee has no label");
    assert_eq!(
        err.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::SpeakerNotIdentified {
                role: SpeakerRole::Coachee,
                labels: vec!["Jim H".to_owned(), "Nobody".to_owned()],
            }
        ))
    );
}

#[test]
fn render_unfiltered_never_fails_on_unresolved_roles() {
    let speakers = vec![speaker("Unknown", None)];
    let segments = vec![segment("Unknown", "Hi.", 0)];
    assert!(render_plain_text(date(), &speakers, &segments, &[]).is_ok());
}

// ---- serde ----

#[test]
fn speaker_role_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&SpeakerRole::Coach).unwrap(),
        "\"coach\""
    );
    assert_eq!(
        serde_json::from_str::<SpeakerRole>("\"coachee\"").unwrap(),
        SpeakerRole::Coachee
    );
    assert!(serde_json::from_str::<SpeakerRole>("\"Coach\"").is_err());
    assert!(serde_json::from_str::<SpeakerRole>("\"bob\"").is_err());
}

#[test]
fn speaker_serializes_role_as_null_when_unresolved() {
    let json = serde_json::to_value(speaker("Guest", None)).unwrap();
    assert_eq!(json, serde_json::json!({"label": "Guest", "role": null}));
}
