//! Backfill against a real SQL engine: which transcriptions are candidates, and that linking
//! attributes speakers without changing a single stored segment.

use chrono::{Duration, Utc};
use entity::meeting_recording::{self, MeetingRecordingStatus};
use entity::transcription::{self, TranscriptionStatus};
use entity::{transcript_participant, transcript_segment};
use meeting_ai::types::transcription::{
    Participant as SpeakerIn, Segment as Rebuilt, Status, Transcription,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, Set};

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_coaching_session, seed_user, within_time_limit};
use crate::transcript_attribution::Attribution;
use crate::transcript_participant::{self as participant_api, MatchSource};
use crate::Id;

/// A transcription in a fresh session, created `minutes_ago` minutes ago.
async fn seed_transcription(
    db: &DatabaseConnection,
    status: TranscriptionStatus,
    minutes_ago: i64,
) -> Id {
    let (session_id, _) = seed_coaching_session(db).await;
    let created = Utc::now() - Duration::minutes(minutes_ago);
    let recording_id = Id::new_v4();

    meeting_recording::ActiveModel {
        id: Set(recording_id),
        coaching_session_id: Set(session_id),
        bot_id: Set(format!("bot-{recording_id}")),
        status: Set(MeetingRecordingStatus::Completed),
        video_url: Set(None),
        audio_url: Set(None),
        duration_seconds: Set(None),
        started_at: Set(None),
        ended_at: Set(None),
        error_message: Set(None),
        created_at: Set(created.into()),
        updated_at: Set(created.into()),
    }
    .insert(db)
    .await
    .expect("the recording is seeded");

    let id = Id::new_v4();
    transcription::ActiveModel {
        id: Set(id),
        coaching_session_id: Set(session_id),
        meeting_recording_id: Set(recording_id),
        external_id: Set(format!("ext-{id}")),
        recall_recording_id: Set(None),
        status: Set(status),
        language_code: Set(None),
        speaker_count: Set(None),
        word_count: Set(None),
        duration_seconds: Set(None),
        confidence: Set(None),
        error_message: Set(None),
        created_at: Set(created.into()),
        updated_at: Set(created.into()),
    }
    .insert(db)
    .await
    .expect("the transcription is seeded");
    id
}

/// A legacy segment: stored before attribution existed, so no participant.
async fn seed_legacy_segment(
    db: &DatabaseConnection,
    transcription_id: Id,
    start_ms: i32,
    text: &str,
) -> Id {
    let id = Id::new_v4();
    transcript_segment::ActiveModel {
        id: Set(id),
        transcription_id: Set(transcription_id),
        participant_id: Set(None),
        speaker_label: Set("Typed Name".to_string()),
        text: Set(text.to_string()),
        start_ms: Set(start_ms),
        end_ms: Set(start_ms + 1000),
        confidence: Set(None),
        sentiment: Set(None),
        created_at: Set(Utc::now().into()),
    }
    .insert(db)
    .await
    .expect("the segment is seeded");
    id
}

async fn seed_existing_participant(db: &DatabaseConnection, transcription_id: Id) {
    transcript_participant::ActiveModel {
        id: Set(Id::new_v4()),
        transcription_id: Set(transcription_id),
        provider_participant_id: Set("100".to_string()),
        display_name: Set(None),
        is_host: Set(None),
        platform: Set(None),
        platform_account_id: Set(None),
        extra_data: Set(None),
        user_id: Set(None),
        match_source: Set(None),
        created_at: Set(Utc::now().into()),
    }
    .insert(db)
    .await
    .expect("the participant is seeded");
}

fn speaker(provider_id: &str, name: &str, is_host: bool) -> SpeakerIn {
    SpeakerIn {
        provider_id: provider_id.to_string(),
        display_name: Some(name.to_string()),
        is_host: Some(is_host),
        platform: None,
        account_id: None,
        extra_data: None,
    }
}

fn rebuilt(participant: Option<&str>, start_ms: i64, text: &str) -> Rebuilt {
    Rebuilt {
        text: text.to_string(),
        speaker: "Typed Name".to_string(),
        participant_id: participant.map(str::to_string),
        start_ms,
        end_ms: start_ms + 1000,
        confidence: 0.0,
        words: vec![],
    }
}

fn result(participants: Vec<SpeakerIn>, segments: Vec<Rebuilt>) -> Transcription {
    Transcription {
        id: "ext".to_string(),
        status: Status::Completed,
        text: None,
        words: vec![],
        segments,
        participants,
        chapters: vec![],
        sentiment_analysis: vec![],
        confidence: None,
        duration_seconds: None,
        language_code: None,
        speaker_count: None,
        error_message: None,
    }
}

/// Every stored segment as `(id, start_ms, text, participant_id)`, ordered by start then id.
async fn segment_rows(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Vec<(Id, i32, String, Option<Id>)> {
    entity_api::transcript_segment::find_by_transcription(db, transcription_id)
        .await
        .expect("segments read")
        .into_iter()
        .map(|s| (s.id, s.start_ms, s.text, s.participant_id))
        .collect()
}

#[tokio::test]
async fn candidates_are_completed_transcriptions_without_participants_oldest_first(
) -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let older = seed_transcription(&db, TranscriptionStatus::Completed, 120).await;
        let newer = seed_transcription(&db, TranscriptionStatus::Completed, 60).await;
        let done = seed_transcription(&db, TranscriptionStatus::Completed, 90).await;
        seed_existing_participant(&db, done).await;
        seed_transcription(&db, TranscriptionStatus::Processing, 30).await;
        seed_transcription(&db, TranscriptionStatus::Failed, 30).await;

        let ids: Vec<Id> = find_candidates(&db, None, None)
            .await?
            .into_iter()
            .map(|t| t.id)
            .collect();

        assert_eq!(ids, vec![older, newer]);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn candidates_respect_since_and_limit() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        seed_transcription(&db, TranscriptionStatus::Completed, 300).await;
        let recent_a = seed_transcription(&db, TranscriptionStatus::Completed, 50).await;
        seed_transcription(&db, TranscriptionStatus::Completed, 40).await;

        let since = Some(Utc::now() - Duration::minutes(100));
        let ids: Vec<Id> = find_candidates(&db, since, Some(1))
            .await?
            .into_iter()
            .map(|t| t.id)
            .collect();

        assert_eq!(ids, vec![recent_a]);
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn stored_segments_come_back_in_start_order() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, TranscriptionStatus::Completed, 10).await;
        let second = seed_legacy_segment(&db, transcription_id, 4000, "Morning.").await;
        let first = seed_legacy_segment(&db, transcription_id, 0, "Good morning.").await;

        let stored = stored_segments(&db, transcription_id).await?;

        let read: Vec<(Id, i32, &str)> = stored
            .iter()
            .map(|s| (s.id, s.start_ms, s.text.as_str()))
            .collect();
        assert_eq!(
            read,
            vec![(first, 0, "Good morning."), (second, 4000, "Morning.")]
        );
        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn linking_attributes_speakers_and_leaves_every_segment_intact() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, TranscriptionStatus::Completed, 10).await;
        let (coach_id, coachee_id) = (seed_user(&db).await, seed_user(&db).await);
        seed_legacy_segment(&db, transcription_id, 0, "Good morning.").await;
        seed_legacy_segment(&db, transcription_id, 4000, "Morning.").await;
        seed_legacy_segment(&db, transcription_id, 9000, "Who am I?").await;
        let before = segment_rows(&db, transcription_id).await;

        let rebuilt_transcript = result(
            vec![
                speaker("100", "Coach Typed", true),
                speaker("200", "Coachee Typed", false),
            ],
            vec![
                rebuilt(Some("100"), 0, "Good morning."),
                rebuilt(Some("200"), 4000, "Morning."),
                rebuilt(Some("999"), 9000, "Who am I?"),
            ],
        );
        let attributions = vec![
            Attribution {
                provider_id: "100".to_string(),
                user_id: Some(coach_id),
                source: Some(MatchSource::Account),
            },
            Attribution {
                provider_id: "200".to_string(),
                user_id: Some(coachee_id),
                source: Some(MatchSource::Elimination),
            },
        ];
        let stored = stored_segments(&db, transcription_id).await?;

        let linked = link_participants(
            &db,
            transcription_id,
            &rebuilt_transcript,
            &attributions,
            &stored,
        )
        .await?;

        assert_eq!(linked, Linked::Done);

        let participants = participant_api::find_by_transcription(&db, transcription_id).await?;
        let row_of = |provider: &str| {
            participants
                .iter()
                .find(|p| p.provider_participant_id == provider)
                .map(|p| (p.id, p.user_id, p.match_source))
        };
        let (coach_row, coach_user, coach_source) = row_of("100").expect("coach row");
        let (coachee_row, coachee_user, coachee_source) = row_of("200").expect("coachee row");
        assert_eq!(
            (coach_user, coach_source),
            (Some(coach_id), Some(MatchSource::Account))
        );
        assert_eq!(
            (coachee_user, coachee_source),
            (Some(coachee_id), Some(MatchSource::Elimination))
        );

        let after = segment_rows(&db, transcription_id).await;
        assert_eq!(after.len(), before.len());
        for (b, a) in before.iter().zip(&after) {
            assert_eq!(
                (b.0, b.1, &b.2),
                (a.0, a.1, &a.2),
                "ids, starts, and text never change"
            );
        }
        let links: Vec<Option<Id>> = after.iter().map(|s| s.3).collect();
        assert_eq!(links, vec![Some(coach_row), Some(coachee_row), None]);

        Ok::<(), Error>(())
    })
    .await
}

#[tokio::test]
async fn a_transcription_that_already_has_participants_is_left_alone() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let transcription_id = seed_transcription(&db, TranscriptionStatus::Completed, 10).await;
        seed_legacy_segment(&db, transcription_id, 0, "Good morning.").await;
        seed_existing_participant(&db, transcription_id).await;
        let stored = stored_segments(&db, transcription_id).await?;

        let linked = link_participants(
            &db,
            transcription_id,
            &result(
                vec![speaker("100", "Coach Typed", true)],
                vec![rebuilt(Some("100"), 0, "Good morning.")],
            ),
            &[],
            &stored,
        )
        .await?;

        assert_eq!(linked, Linked::AlreadyDone);
        assert_eq!(
            participant_api::find_by_transcription(&db, transcription_id)
                .await?
                .len(),
            1
        );
        assert_eq!(segment_rows(&db, transcription_id).await[0].3, None);

        Ok::<(), Error>(())
    })
    .await
}
