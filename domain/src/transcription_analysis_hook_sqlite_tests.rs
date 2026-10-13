//! The pending analysis written with a completed transcript, checked against a real SQL engine.

use entity::{transcript_analysis, transcript_segment};
use meeting_ai::types::transcription::{Segment as SegmentIn, Status, Transcription};
use sea_orm::{ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter};

use super::*;
use crate::error::Error;
use crate::test_utils::sqlite::{database, seed_transcription, within_time_limit};

/// A seeded transcription with no segments yet.
async fn fixture() -> (DatabaseConnection, Model) {
    let db = database().await;
    let id = seed_transcription(&db, 0).await.transcription_id;
    let transcription = entity::transcription::Entity::find_by_id(id)
        .one(&db)
        .await
        .expect("the transcription is read")
        .expect("the transcription exists");
    (db, transcription)
}

fn transcript() -> Transcription {
    Transcription {
        id: "ext-1".to_string(),
        status: Status::Completed,
        text: None,
        words: vec![],
        segments: vec![SegmentIn {
            text: "I will send the notes.".to_string(),
            speaker: "A".to_string(),
            participant_id: None,
            start_ms: 0,
            end_ms: 1000,
            confidence: 0.0,
            words: vec![],
        }],
        participants: vec![],
        chapters: vec![],
        sentiment_analysis: vec![],
        confidence: None,
        duration_seconds: None,
        language_code: None,
        speaker_count: None,
        error_message: None,
    }
}

async fn analyses(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Vec<transcript_analysis::Model> {
    transcript_analysis::Entity::find()
        .filter(transcript_analysis::Column::TranscriptionId.eq(transcription_id))
        .all(db)
        .await
        .expect("the analyses are read")
}

async fn segment_count(db: &DatabaseConnection, transcription_id: Id) -> usize {
    transcript_segment::Entity::find()
        .filter(transcript_segment::Column::TranscriptionId.eq(transcription_id))
        .all(db)
        .await
        .expect("the segments are read")
        .len()
}

#[tokio::test]
async fn completing_with_a_target_writes_one_pending_analysis_with_the_segments(
) -> Result<(), Error> {
    within_time_limit(async {
        let (db, transcription) = fixture().await;

        persist_completion_with(&db, &transcription, &transcript(), &[], &[Target::Action]).await?;

        let rows = analyses(&db, transcription.id).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].target, Target::Action);
        assert_eq!(rows[0].status, transcript_analysis::Status::Pending);
        assert_eq!(rows[0].attempts, 0);
        assert_eq!(segment_count(&db, transcription.id).await, 1);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn completing_without_targets_writes_no_analysis() -> Result<(), Error> {
    within_time_limit(async {
        let (db, transcription) = fixture().await;

        persist_completion(&db, &transcription, &transcript(), &[]).await?;

        assert!(analyses(&db, transcription.id).await.is_empty());
        assert_eq!(segment_count(&db, transcription.id).await, 1);
        Ok(())
    })
    .await
}

#[tokio::test]
async fn a_failed_analysis_insert_rolls_back_the_segments() -> Result<(), Error> {
    within_time_limit(async {
        let (db, transcription) = fixture().await;
        // Schema sync builds tables from entities, so mirror the migration's unique constraint.
        db.execute_unprepared(
            "CREATE UNIQUE INDEX refactor_platform.transcript_analyses_transcription_target_unique \
             ON transcript_analyses (transcription_id, target)",
        )
        .await
        .expect("the unique index is created");
        analysis_api::create_pending(&db, transcription.id, Target::Action).await?;

        let result =
            persist_completion_with(&db, &transcription, &transcript(), &[], &[Target::Action])
                .await;

        assert!(result.is_err());
        assert_eq!(segment_count(&db, transcription.id).await, 0);
        assert_eq!(analyses(&db, transcription.id).await.len(), 1);
        Ok(())
    })
    .await
}
