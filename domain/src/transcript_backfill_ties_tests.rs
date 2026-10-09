//! Frozen acceptance tests: segments that start in the same millisecond are matched by content,
//! not position, and are only refused when they cannot be told apart.
//! Written by the overseer before implementation; read-only during the build.

use meeting_ai::types::transcription::Segment as Rebuilt;

use super::*;
use crate::Id;

fn stored(id: u128, start_ms: i32, text: &str) -> StoredSegment {
    StoredSegment {
        id: Id::from_u128(id),
        start_ms,
        text: text.to_string(),
    }
}

fn rebuilt(participant: &str, start_ms: i64, text: &str) -> Rebuilt {
    Rebuilt {
        text: text.to_string(),
        speaker: "Anyone".to_string(),
        participant_id: Some(participant.to_string()),
        start_ms,
        end_ms: start_ms + 500,
        confidence: 0.0,
        words: vec![],
    }
}

/// Production shape: two people speak in the same millisecond, and the stored rows broke the tie by
/// row id while the rebuild kept the provider's order.
fn swapped_tie() -> (Vec<StoredSegment>, Vec<Rebuilt>) {
    (
        vec![
            stored(1, 0, "Good morning."),
            stored(2, 1000, "Yes."),
            stored(3, 1000, "Right, so"),
            stored(4, 2000, "Okay."),
        ],
        vec![
            rebuilt("100", 0, "Good morning."),
            rebuilt("100", 1000, "Right, so"),
            rebuilt("200", 1000, "Yes."),
            rebuilt("200", 2000, "Okay."),
        ],
    )
}

#[test]
fn segments_swapped_within_a_shared_start_still_match() {
    let (stored, rebuilt) = swapped_tie();

    assert!(segments_match(&stored, &rebuilt).is_ok());
}

#[test]
fn pairing_follows_content_not_position() {
    let (stored, rebuilt) = swapped_tie();

    let pairs = pair_segments(&stored, &rebuilt).expect("the tie is distinguishable by text");

    let by_stored: Vec<(Id, Option<&str>)> = pairs
        .iter()
        .map(|(id, participant)| (*id, participant.as_deref()))
        .collect();
    assert_eq!(
        by_stored,
        vec![
            (Id::from_u128(1), Some("100")),
            (Id::from_u128(2), Some("200")),
            (Id::from_u128(3), Some("100")),
            (Id::from_u128(4), Some("200")),
        ]
    );
}

#[test]
fn identical_lines_from_different_people_at_one_start_are_refused() {
    let stored = vec![stored(1, 1000, "Yeah."), stored(2, 1000, "Yeah.")];
    let rebuilt = vec![rebuilt("100", 1000, "Yeah."), rebuilt("200", 1000, "Yeah.")];

    let detail = pair_segments(&stored, &rebuilt).expect_err("cannot tell who said which");

    assert!(!detail.contains("Yeah"), "{detail}");
    assert!(!detail.contains(','), "{detail}");
}

#[test]
fn identical_lines_from_the_same_person_at_one_start_pair_to_that_person() {
    let stored = vec![stored(1, 1000, "Yeah."), stored(2, 1000, "Yeah.")];
    let rebuilt = vec![rebuilt("100", 1000, "Yeah."), rebuilt("100", 1000, "Yeah.")];

    let pairs = pair_segments(&stored, &rebuilt).expect("same speaker either way");

    assert!(pairs.iter().all(|(_, p)| p.as_deref() == Some("100")));
    assert_eq!(pairs.len(), 2);
}

#[test]
fn content_that_differs_is_still_refused() {
    let stored = vec![stored(1, 1000, "Yes."), stored(2, 1000, "Right, so")];
    let rebuilt = vec![
        rebuilt("100", 1000, "Right, so"),
        rebuilt("200", 1000, "No."),
    ];

    assert!(segments_match(&stored, &rebuilt).is_err());
    assert!(pair_segments(&stored, &rebuilt).is_err());
}

#[test]
fn every_stored_segment_is_paired_exactly_once() {
    let (stored, rebuilt) = swapped_tie();

    let pairs = pair_segments(&stored, &rebuilt).expect("pairs");

    let mut ids: Vec<Id> = pairs.iter().map(|(id, _)| *id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), stored.len());
}
