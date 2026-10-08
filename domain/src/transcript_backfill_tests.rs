//! Frozen acceptance tests for the backfill's exact-match rule.
//! Written by the overseer before implementation; read-only during the build.

use meeting_ai::types::transcription::Segment as Rebuilt;

use super::*;
use crate::Id;

fn stored(start_ms: i32, text: &str) -> StoredSegment {
    StoredSegment {
        id: Id::new_v4(),
        start_ms,
        text: text.to_string(),
    }
}

fn rebuilt(start_ms: i64, text: &str) -> Rebuilt {
    Rebuilt {
        text: text.to_string(),
        speaker: "Anyone".to_string(),
        participant_id: Some("100".to_string()),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: 0.0,
        words: vec![],
    }
}

#[test]
fn identical_segments_match() {
    let stored = vec![stored(0, "Good morning."), stored(4000, "Morning.")];
    let rebuilt = vec![rebuilt(0, "Good morning."), rebuilt(4000, "Morning.")];

    assert!(segments_match(&stored, &rebuilt).is_ok());
}

#[test]
fn no_segments_on_either_side_match() {
    assert!(segments_match(&[], &[]).is_ok());
}

#[test]
fn a_different_count_does_not_match() {
    let stored = vec![stored(0, "Good morning.")];
    let rebuilt = vec![rebuilt(0, "Good morning."), rebuilt(4000, "Morning.")];

    let detail = segments_match(&stored, &rebuilt).expect_err("counts differ");

    assert!(detail.contains('1') && detail.contains('2'), "{detail}");
}

#[test]
fn different_text_does_not_match_and_never_quotes_it() {
    let stored = vec![stored(0, "Good morning."), stored(4000, "Private words.")];
    let rebuilt = vec![rebuilt(0, "Good morning."), rebuilt(4000, "Other words.")];

    let detail = segments_match(&stored, &rebuilt).expect_err("text differs");

    assert!(!detail.contains("Private"), "{detail}");
    assert!(!detail.contains("Other"), "{detail}");
    assert!(
        !detail.contains(','),
        "the detail lands in a CSV column: {detail}"
    );
}

#[test]
fn a_different_start_does_not_match() {
    let stored = vec![stored(0, "Good morning."), stored(4000, "Morning.")];
    let rebuilt = vec![rebuilt(0, "Good morning."), rebuilt(4001, "Morning.")];

    assert!(segments_match(&stored, &rebuilt).is_err());
}

#[test]
fn speaker_labels_are_not_compared() {
    let stored = vec![stored(0, "Good morning.")];
    let mut renamed = rebuilt(0, "Good morning.");
    renamed.speaker = "Someone Else".to_string();

    assert!(segments_match(&stored, &[renamed]).is_ok());
}
