//! Frozen acceptance tests for transcript labeling and plain-text rendering.
//! Written by the overseer before implementation; read-only during the build.

use chrono::Utc;

use super::*;
use crate::error::{DomainErrorKind, EntityErrorKind, InternalErrorKind};
use crate::transcript_participant::{MatchSource, Model as Participant};
use crate::Id;

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

/// Profile name `Jim H`.
fn coach() -> users::Model {
    user("Jim", "Hodapp", Some("Jim H"))
}

/// Profile name `Caleb Bourg`.
fn coachee() -> users::Model {
    user("Caleb", "Bourg", None)
}

fn participant(
    name: Option<&str>,
    account: Option<&str>,
    attributed_to: Option<(Id, MatchSource)>,
) -> Participant {
    Participant {
        id: Id::new_v4(),
        transcription_id: Id::nil(),
        provider_participant_id: Id::new_v4().to_string(),
        display_name: name.map(str::to_owned),
        is_host: None,
        platform: None,
        platform_account_id: account.map(str::to_owned),
        extra_data: None,
        user_id: attributed_to.map(|(user_id, _)| user_id),
        match_source: attributed_to.map(|(_, source)| source),
        created_at: Utc::now().into(),
    }
}

/// A segment as stored: `raw_label` is the meeting name the provider reported.
fn spoken(by: Option<&Participant>, raw_label: &str, text: &str, start_ms: i32) -> Segment {
    Segment {
        id: Id::new_v4(),
        transcription_id: Id::nil(),
        participant_id: by.map(|p| p.id),
        speaker_label: raw_label.to_owned(),
        text: text.to_owned(),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: None,
        sentiment: None,
        created_at: Utc::now().into(),
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

fn segment_labels(labeled: &Labeled) -> Vec<&str> {
    labeled
        .segments
        .iter()
        .map(|s| s.speaker_label.as_str())
        .collect()
}

/// Coach (raw `J. Hodapp`), coachee (raw `CB`), and a guest (raw `Pat`).
fn three_speaker_transcript(
    coach: &users::Model,
    coachee: &users::Model,
) -> (Vec<Participant>, Vec<Segment>) {
    let coach_p = participant(
        Some("J. Hodapp"),
        None,
        Some((coach.id, MatchSource::Account)),
    );
    let coachee_p = participant(
        Some("CB"),
        None,
        Some((coachee.id, MatchSource::Elimination)),
    );
    let guest_p = participant(Some("Pat"), None, None);
    let segments = vec![
        spoken(Some(&coach_p), "J. Hodapp", "Good morning.", 0),
        spoken(Some(&coachee_p), "CB", "Morning.", 4000),
        spoken(Some(&guest_p), "Pat", "Hi both.", 9000),
        spoken(Some(&coach_p), "J. Hodapp", "Wrapping up.", 3_735_000),
    ];
    (vec![coach_p, coachee_p, guest_p], segments)
}

// ---- label_transcript ----

#[test]
fn attributed_speakers_show_profile_names_and_their_roles() {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);

    let labeled = label_transcript(&participants, &segments, &coach, &coachee);

    assert_eq!(
        labels(&labeled.speakers),
        vec!["Jim H", "Caleb Bourg", "Pat"]
    );
    assert_eq!(
        roles(&labeled.speakers),
        vec![Some(SpeakerRole::Coach), Some(SpeakerRole::Coachee), None]
    );
    assert_eq!(
        segment_labels(&labeled),
        vec!["Jim H", "Caleb Bourg", "Pat", "Jim H"]
    );
}

#[test]
fn each_segment_carries_its_user_and_role() {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);

    let labeled = label_transcript(&participants, &segments, &coach, &coachee);

    let attribution: Vec<(Option<Id>, Option<SpeakerRole>)> = labeled
        .segments
        .iter()
        .map(|s| (s.speaker_user_id, s.speaker_role))
        .collect();
    assert_eq!(
        attribution,
        vec![
            (Some(coach.id), Some(SpeakerRole::Coach)),
            (Some(coachee.id), Some(SpeakerRole::Coachee)),
            (None, None),
            (Some(coach.id), Some(SpeakerRole::Coach)),
        ]
    );
}

#[test]
fn every_segment_label_names_exactly_one_speaker_with_the_same_role() {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);

    let labeled = label_transcript(&participants, &segments, &coach, &coachee);

    for segment in &labeled.segments {
        let matching: Vec<&Speaker> = labeled
            .speakers
            .iter()
            .filter(|speaker| speaker.label == segment.speaker_label)
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "label {} must be unique",
            segment.speaker_label
        );
        assert_eq!(matching[0].role, segment.speaker_role);
    }
}

#[test]
fn nameless_speakers_are_numbered_guests_in_speaking_order() {
    let (coach, coachee) = (coach(), coachee());
    let first = participant(None, None, None);
    let second = participant(None, None, None);
    let segments = vec![
        spoken(Some(&second), "200", "I spoke first.", 0),
        spoken(Some(&first), "100", "I spoke second.", 5000),
    ];

    let labeled = label_transcript(&[first, second], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Guest 1", "Guest 2"]);
    assert_eq!(segment_labels(&labeled), vec!["Guest 1", "Guest 2"]);
}

#[test]
fn a_typed_name_that_matches_a_profile_name_gets_a_suffix() {
    let (coach, coachee) = (coach(), coachee());
    let impostor = participant(Some("Jim H"), None, None);
    let real = participant(
        Some("J. Hodapp"),
        None,
        Some((coach.id, MatchSource::Account)),
    );
    let segments = vec![
        spoken(Some(&impostor), "Jim H", "I spoke first.", 0),
        spoken(Some(&real), "J. Hodapp", "I am the coach.", 5000),
    ];

    let labeled = label_transcript(&[impostor, real], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Jim H (2)", "Jim H"]);
    assert_eq!(
        roles(&labeled.speakers),
        vec![None, Some(SpeakerRole::Coach)]
    );
}

#[test]
fn two_different_people_with_one_typed_name_are_told_apart() {
    let (coach, coachee) = (coach(), coachee());
    let first = participant(Some("Pat"), Some("acct-1"), None);
    let second = participant(Some("Pat"), Some("acct-2"), None);
    let segments = vec![
        spoken(Some(&first), "Pat", "One.", 0),
        spoken(Some(&second), "Pat", "Two.", 5000),
    ];

    let labeled = label_transcript(&[first, second], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Pat", "Pat (2)"]);
}

#[test]
fn a_typed_guest_number_does_not_collide_with_a_generated_one() {
    let (coach, coachee) = (coach(), coachee());
    let typed = participant(Some("Guest 1"), None, None);
    let nameless = participant(None, None, None);
    let segments = vec![
        spoken(Some(&typed), "Guest 1", "Typed.", 0),
        spoken(Some(&nameless), "300", "Nameless.", 5000),
    ];

    let labeled = label_transcript(&[typed, nameless], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Guest 1", "Guest 1 (2)"]);
}

#[test]
fn an_attributed_rejoin_is_one_speaker() {
    let (coach, coachee) = (coach(), coachee());
    let before = participant(
        Some("CB"),
        None,
        Some((coachee.id, MatchSource::Elimination)),
    );
    let after = participant(
        Some("CB"),
        None,
        Some((coachee.id, MatchSource::Elimination)),
    );
    let segments = vec![
        spoken(Some(&before), "CB", "Before.", 0),
        spoken(Some(&after), "CB", "After.", 60_000),
    ];

    let labeled = label_transcript(&[before, after], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Caleb Bourg"]);
    assert_eq!(segment_labels(&labeled), vec!["Caleb Bourg", "Caleb Bourg"]);
}

#[test]
fn an_unattributed_rejoin_under_one_name_is_one_speaker() {
    let (coach, coachee) = (coach(), coachee());
    let before = participant(Some("Pat"), None, None);
    let after = participant(Some("Pat"), None, None);
    let segments = vec![
        spoken(Some(&before), "Pat", "Before.", 0),
        spoken(Some(&after), "Pat", "After.", 60_000),
    ];

    let labeled = label_transcript(&[before, after], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Pat"]);
}

#[test]
fn one_account_under_two_names_is_one_speaker_named_as_first_seen() {
    let (coach, coachee) = (coach(), coachee());
    let first = participant(Some("Pat"), Some("acct-1"), None);
    let renamed = participant(Some("Patricia"), Some("acct-1"), None);
    let segments = vec![
        spoken(Some(&first), "Pat", "Before.", 0),
        spoken(Some(&renamed), "Patricia", "After.", 60_000),
    ];

    let labeled = label_transcript(&[first, renamed], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Pat"]);
    assert_eq!(segment_labels(&labeled), vec!["Pat", "Pat"]);
}

#[test]
fn roles_follow_the_relationship_as_it_is_now() {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);

    // The same stored attribution, read after the two people swapped roles.
    let labeled = label_transcript(&participants, &segments, &coachee, &coach);

    assert_eq!(
        labels(&labeled.speakers),
        vec!["Jim H", "Caleb Bourg", "Pat"]
    );
    assert_eq!(
        roles(&labeled.speakers),
        vec![Some(SpeakerRole::Coachee), Some(SpeakerRole::Coach), None]
    );
}

#[test]
fn a_user_outside_the_relationship_is_never_exposed() {
    let (coach, coachee) = (coach(), coachee());
    let stranger = participant(
        Some("Someone"),
        None,
        Some((Id::new_v4(), MatchSource::Account)),
    );
    let segments = vec![spoken(Some(&stranger), "Someone", "Hello.", 0)];

    let labeled = label_transcript(&[stranger], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Someone"]);
    assert_eq!(roles(&labeled.speakers), vec![None]);
    assert_eq!(labeled.segments[0].speaker_user_id, None);
}

#[test]
fn transcripts_without_participants_keep_their_stored_labels() {
    let (coach, coachee) = (coach(), coachee());
    let segments = vec![
        spoken(None, "Jim H", "Legacy line.", 0),
        spoken(None, "Unknown", "Another.", 5000),
        spoken(None, "Jim H", "Again.", 9000),
    ];

    let labeled = label_transcript(&[], &segments, &coach, &coachee);

    assert_eq!(labels(&labeled.speakers), vec!["Jim H", "Unknown"]);
    assert_eq!(roles(&labeled.speakers), vec![None, None]);
    assert_eq!(segment_labels(&labeled), vec!["Jim H", "Unknown", "Jim H"]);
}

#[test]
fn segments_are_ordered_by_start_then_id() {
    let (coach, coachee) = (coach(), coachee());
    let guest = participant(Some("Pat"), None, None);
    let mut segments = vec![
        spoken(Some(&guest), "Pat", "late", 5000),
        spoken(Some(&guest), "Pat", "tie-b", 1000),
        spoken(Some(&guest), "Pat", "tie-a", 1000),
    ];
    segments[1].id = Id::from_u128(2);
    segments[2].id = Id::from_u128(1);

    let labeled = label_transcript(&[guest], &segments, &coach, &coachee);

    let texts: Vec<&str> = labeled.segments.iter().map(|s| s.text.as_str()).collect();
    assert_eq!(texts, vec!["tie-a", "tie-b", "late"]);
}

#[test]
fn no_segments_label_nothing() {
    let (coach, coachee) = (coach(), coachee());

    let labeled = label_transcript(&[], &[], &coach, &coachee);

    assert!(labeled.speakers.is_empty());
    assert!(labeled.segments.is_empty());
}

// ---- timestamps ----

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
    assert_eq!(format_timestamp(1_999), "0:01");
}

// ---- render_plain_text ----

fn three_speaker_labeled() -> Labeled {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);
    label_transcript(&participants, &segments, &coach, &coachee)
}

#[test]
fn render_unfiltered_pins_the_exact_body_and_filename() {
    let rendered = render_plain_text(date(), &three_speaker_labeled(), &[]).expect("renders");
    assert_eq!(
        rendered.body,
        "Coaching session transcript\n\
         Date: 2026-09-21\n\
         Speakers: Jim H, Caleb Bourg, Pat\n\
         \n\
         [0:00] Jim H: Good morning.\n\
         [0:04] Caleb Bourg: Morning.\n\
         [0:09] Pat: Hi both.\n\
         [1:02:15] Jim H: Wrapping up.\n"
    );
    assert_eq!(rendered.filename, "transcript-2026-09-21.txt");
}

#[test]
fn render_filtered_to_coach_keeps_only_coach_lines() {
    let rendered = render_plain_text(date(), &three_speaker_labeled(), &[SpeakerRole::Coach])
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
    assert_eq!(rendered.filename, "transcript-2026-09-21-jim-h.txt");
}

#[test]
fn render_filtered_to_both_roles_drops_guests() {
    let rendered = render_plain_text(
        date(),
        &three_speaker_labeled(),
        &[SpeakerRole::Coachee, SpeakerRole::Coach],
    )
    .expect("renders");
    assert!(rendered.body.contains("Speakers: Jim H, Caleb Bourg\n"));
    assert!(!rendered.body.contains("Pat"));
    assert_eq!(
        rendered.filename,
        "transcript-2026-09-21-jim-h-caleb-bourg.txt"
    );
}

// ---- filtered filenames ----

/// One attributed coach labeled `name`, so the filename shows how that label is slugged.
fn coach_named(display: &str) -> Labeled {
    let coach = user("Jim", "Hodapp", Some(display));
    let coachee = coachee();
    let coach_p = participant(Some("typed"), None, Some((coach.id, MatchSource::Account)));
    let segments = vec![spoken(Some(&coach_p), "typed", "Hello.", 0)];
    label_transcript(&[coach_p], &segments, &coach, &coachee)
}

#[test]
fn a_filtered_filename_names_the_selected_people() {
    let coach_only = render_plain_text(date(), &three_speaker_labeled(), &[SpeakerRole::Coach])
        .expect("renders");
    let coachee_only = render_plain_text(date(), &three_speaker_labeled(), &[SpeakerRole::Coachee])
        .expect("renders");

    assert_eq!(coach_only.filename, "transcript-2026-09-21-jim-h.txt");
    assert_eq!(
        coachee_only.filename,
        "transcript-2026-09-21-caleb-bourg.txt"
    );
    assert_ne!(coach_only.filename, coachee_only.filename);
}

#[test]
fn a_filtered_filename_slugs_punctuation_and_keeps_non_ascii_letters() {
    let rendered = render_plain_text(
        date(),
        &coach_named("José  O'Brien-Smith!"),
        &[SpeakerRole::Coach],
    )
    .expect("renders");

    assert_eq!(
        rendered.filename,
        "transcript-2026-09-21-josé-o-brien-smith.txt"
    );
}

#[test]
fn a_name_with_nothing_usable_falls_back_to_the_role() {
    let rendered =
        render_plain_text(date(), &coach_named("🙂 !!"), &[SpeakerRole::Coach]).expect("renders");

    assert_eq!(rendered.filename, "transcript-2026-09-21-coach.txt");
}

#[test]
fn names_that_slug_alike_carry_their_role() {
    let coach = user("Ann", "Lee", Some("Ann-Marie"));
    let coachee = user("Ann", "Cole", Some("Ann Marie"));
    let coach_p = participant(Some("A"), None, Some((coach.id, MatchSource::Account)));
    let coachee_p = participant(
        Some("B"),
        None,
        Some((coachee.id, MatchSource::Elimination)),
    );
    let segments = vec![
        spoken(Some(&coach_p), "A", "Hello.", 0),
        spoken(Some(&coachee_p), "B", "Hi.", 1000),
    ];
    let labeled = label_transcript(&[coach_p, coachee_p], &segments, &coach, &coachee);

    let filename = |filter: &[SpeakerRole]| {
        render_plain_text(date(), &labeled, filter)
            .expect("renders")
            .filename
    };

    assert_eq!(
        filename(&[SpeakerRole::Coach]),
        "transcript-2026-09-21-ann-marie-coach.txt"
    );
    assert_eq!(
        filename(&[SpeakerRole::Coachee]),
        "transcript-2026-09-21-ann-marie-coachee.txt"
    );
    assert_eq!(
        filename(&[SpeakerRole::Coach, SpeakerRole::Coachee]),
        "transcript-2026-09-21-ann-marie-coach-ann-marie-coachee.txt"
    );
}

#[test]
fn render_filters_by_attribution_not_by_label_text() {
    let (coach, coachee) = (coach(), coachee());
    let impostor = participant(Some("Jim H"), None, None);
    let real = participant(
        Some("J. Hodapp"),
        None,
        Some((coach.id, MatchSource::Account)),
    );
    let segments = vec![
        spoken(Some(&impostor), "Jim H", "Impostor line.", 0),
        spoken(Some(&real), "J. Hodapp", "Coach line.", 5000),
    ];
    let labeled = label_transcript(&[impostor, real], &segments, &coach, &coachee);

    let rendered = render_plain_text(date(), &labeled, &[SpeakerRole::Coach]).expect("renders");

    assert!(rendered.body.contains("[0:05] Jim H: Coach line.\n"));
    assert!(!rendered.body.contains("Impostor line."));
}

#[test]
fn render_header_lists_only_speakers_with_surviving_lines() {
    let (coach, coachee) = (coach(), coachee());
    let coach_p = participant(
        Some("J. Hodapp"),
        None,
        Some((coach.id, MatchSource::Account)),
    );
    let quiet = participant(Some("Pat"), None, None);
    let segments = vec![
        spoken(Some(&coach_p), "J. Hodapp", "Hello.", 0),
        spoken(Some(&quiet), "Pat", "   ", 2000),
    ];
    let labeled = label_transcript(&[coach_p, quiet], &segments, &coach, &coachee);

    let rendered = render_plain_text(date(), &labeled, &[]).expect("renders");

    assert!(rendered.body.contains("Speakers: Jim H\n"));
    assert!(!rendered.body.contains("Pat"));
}

#[test]
fn render_trims_surrounding_whitespace_from_text() {
    let (coach, coachee) = (coach(), coachee());
    let guest = participant(Some("Pat"), None, None);
    let segments = vec![spoken(Some(&guest), "Pat", "  Hello there.  ", 0)];
    let labeled = label_transcript(&[guest], &segments, &coach, &coachee);

    let rendered = render_plain_text(date(), &labeled, &[]).expect("renders");

    assert!(rendered.body.ends_with("[0:00] Pat: Hello there.\n"));
}

#[test]
fn render_with_no_segments_yields_header_only() {
    let (coach, coachee) = (coach(), coachee());
    let labeled = label_transcript(&[], &[], &coach, &coachee);

    let rendered = render_plain_text(date(), &labeled, &[]).expect("renders");

    assert_eq!(
        rendered.body,
        "Coaching session transcript\nDate: 2026-09-21\nSpeakers: \n\n"
    );
}

#[test]
fn render_fails_when_a_requested_role_has_no_speaker() {
    let (coach, coachee) = (coach(), coachee());
    let coach_p = participant(
        Some("J. Hodapp"),
        None,
        Some((coach.id, MatchSource::Account)),
    );
    let guest = participant(Some("Pat"), None, None);
    let segments = vec![
        spoken(Some(&coach_p), "J. Hodapp", "Hello.", 0),
        spoken(Some(&guest), "Pat", "Hi.", 2000),
    ];
    let labeled = label_transcript(&[coach_p, guest], &segments, &coach, &coachee);

    let err = render_plain_text(
        date(),
        &labeled,
        &[SpeakerRole::Coach, SpeakerRole::Coachee],
    )
    .expect_err("nobody is attributed to the coachee");

    assert_eq!(
        err.error_kind,
        DomainErrorKind::Internal(InternalErrorKind::Entity(
            EntityErrorKind::SpeakerNotIdentified {
                role: SpeakerRole::Coachee,
                labels: vec!["Jim H".to_owned(), "Pat".to_owned()],
            }
        ))
    );
}

#[test]
fn render_unfiltered_never_fails_without_attribution() {
    let (coach, coachee) = (coach(), coachee());
    let segments = vec![spoken(None, "Unknown", "Hi.", 0)];
    let labeled = label_transcript(&[], &segments, &coach, &coachee);

    assert!(render_plain_text(date(), &labeled, &[]).is_ok());
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
fn speaker_serializes_role_as_null_when_unattributed() {
    let json = serde_json::to_value(Speaker {
        label: "Pat".to_owned(),
        role: None,
    })
    .unwrap();
    assert_eq!(json, serde_json::json!({"label": "Pat", "role": null}));
}

#[test]
fn a_labeled_segment_serializes_attribution_and_hides_raw_provider_data() {
    let (coach, coachee) = (coach(), coachee());
    let (participants, segments) = three_speaker_transcript(&coach, &coachee);
    let labeled = label_transcript(&participants, &segments, &coach, &coachee);

    let coach_line = serde_json::to_value(&labeled.segments[0]).unwrap();
    let guest_line = serde_json::to_value(&labeled.segments[2]).unwrap();

    assert_eq!(coach_line["speaker_label"], "Jim H");
    assert_eq!(coach_line["speaker_user_id"], serde_json::json!(coach.id));
    assert_eq!(coach_line["speaker_role"], "coach");
    assert_eq!(coach_line["text"], "Good morning.");
    assert_eq!(coach_line["start_ms"], 0);
    assert_eq!(guest_line["speaker_user_id"], serde_json::Value::Null);
    assert_eq!(guest_line["speaker_role"], serde_json::Value::Null);
    for hidden in ["participant_id", "display_name", "platform_account_id"] {
        assert!(
            coach_line.get(hidden).is_none(),
            "{hidden} must not be serialized"
        );
    }
    for kept in [
        "id",
        "transcription_id",
        "end_ms",
        "confidence",
        "sentiment",
        "created_at",
    ] {
        assert!(coach_line.get(kept).is_some(), "{kept} must be serialized");
    }
}
