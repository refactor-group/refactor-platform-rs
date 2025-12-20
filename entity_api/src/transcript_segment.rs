//! CRUD operations for transcript_segments table.

use super::error::{EntityApiErrorKind, Error};
use entity::sentiment::Sentiment;
use entity::transcript_segments::{ActiveModel, Entity, Model};
use entity::Id;
use log::*;
use sea_orm::{entity::prelude::*, ActiveValue::Set, DatabaseConnection, QueryOrder, TryIntoModel};

/// Input for creating a transcript segment
#[derive(Debug, Clone)]
pub struct SegmentInput {
    pub speaker_label: String,
    pub text: String,
    pub start_time_ms: i64,
    pub end_time_ms: i64,
    pub confidence: Option<f64>,
    pub sentiment: Option<Sentiment>,
}

/// Creates a new transcript segment
pub async fn create(
    db: &DatabaseConnection,
    transcription_id: Id,
    input: SegmentInput,
) -> Result<Model, Error> {
    debug!(
        "Creating transcript segment for transcription: {transcription_id}, speaker: {}",
        input.speaker_label
    );

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        transcription_id: Set(transcription_id),
        speaker_label: Set(input.speaker_label),
        speaker_user_id: Set(None),
        text: Set(input.text),
        start_time_ms: Set(input.start_time_ms),
        end_time_ms: Set(input.end_time_ms),
        confidence: Set(input.confidence),
        sentiment: Set(input.sentiment),
        created_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Creates multiple transcript segments in batch
pub async fn create_batch(
    db: &DatabaseConnection,
    transcription_id: Id,
    segments: Vec<SegmentInput>,
) -> Result<Vec<Model>, Error> {
    debug!(
        "Creating {} transcript segments for transcription: {transcription_id}",
        segments.len()
    );

    let now = chrono::Utc::now();

    let mut created = Vec::with_capacity(segments.len());

    for segment in segments {
        let active_model = ActiveModel {
            transcription_id: Set(transcription_id),
            speaker_label: Set(segment.speaker_label),
            speaker_user_id: Set(None),
            text: Set(segment.text),
            start_time_ms: Set(segment.start_time_ms),
            end_time_ms: Set(segment.end_time_ms),
            confidence: Set(segment.confidence),
            sentiment: Set(segment.sentiment),
            created_at: Set(now.into()),
            ..Default::default()
        };

        let model = active_model.save(db).await?.try_into_model()?;
        created.push(model);
    }

    Ok(created)
}

/// Finds a transcript segment by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds all transcript segments for a transcription, ordered by start time
pub async fn find_by_transcription_id(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::transcript_segments::Column::TranscriptionId.eq(transcription_id))
        .order_by_asc(entity::transcript_segments::Column::StartTimeMs)
        .all(db)
        .await?)
}

/// Updates the speaker user ID for a segment (for speaker identification)
pub async fn update_speaker_user_id(
    db: &DatabaseConnection,
    id: Id,
    speaker_user_id: Option<Id>,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating speaker user ID for segment: {id}");

            let active_model = ActiveModel {
                id: Set(existing.id),
                transcription_id: Set(existing.transcription_id),
                speaker_label: Set(existing.speaker_label),
                speaker_user_id: Set(speaker_user_id),
                text: Set(existing.text),
                start_time_ms: Set(existing.start_time_ms),
                end_time_ms: Set(existing.end_time_ms),
                confidence: Set(existing.confidence),
                sentiment: Set(existing.sentiment),
                created_at: Set(existing.created_at),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }),
    }
}

/// Deletes all transcript segments for a transcription
pub async fn delete_by_transcription_id(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Result<u64, Error> {
    let result = Entity::delete_many()
        .filter(entity::transcript_segments::Column::TranscriptionId.eq(transcription_id))
        .exec(db)
        .await?;

    Ok(result.rows_affected)
}
