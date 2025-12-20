//! CRUD operations for transcriptions table.

use super::error::{EntityApiErrorKind, Error};
use entity::transcription_status::TranscriptionStatus;
use entity::transcriptions::{ActiveModel, Entity, Model};
use entity::Id;
use log::*;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

/// Creates a new transcription record
pub async fn create(db: &DatabaseConnection, meeting_recording_id: Id) -> Result<Model, Error> {
    debug!("Creating new transcription for recording: {meeting_recording_id}");

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        meeting_recording_id: Set(meeting_recording_id),
        status: Set(TranscriptionStatus::Pending),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Updates an existing transcription record
pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating transcription: {id}");

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                meeting_recording_id: Unchanged(existing.meeting_recording_id),
                assemblyai_transcript_id: Set(model.assemblyai_transcript_id),
                status: Set(model.status),
                full_text: Set(model.full_text),
                summary: Set(model.summary),
                confidence_score: Set(model.confidence_score),
                word_count: Set(model.word_count),
                language_code: Set(model.language_code),
                error_message: Set(model.error_message),
                created_at: Unchanged(existing.created_at),
                updated_at: Set(chrono::Utc::now().into()),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            debug!("Transcription with id {id} not found");
            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

/// Updates the status of a transcription
pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: TranscriptionStatus,
    error_message: Option<String>,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating transcription status to {:?}: {id}", status);

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                meeting_recording_id: Unchanged(existing.meeting_recording_id),
                assemblyai_transcript_id: Unchanged(existing.assemblyai_transcript_id),
                status: Set(status),
                full_text: Unchanged(existing.full_text),
                summary: Unchanged(existing.summary),
                confidence_score: Unchanged(existing.confidence_score),
                word_count: Unchanged(existing.word_count),
                language_code: Unchanged(existing.language_code),
                error_message: Set(error_message),
                created_at: Unchanged(existing.created_at),
                updated_at: Set(chrono::Utc::now().into()),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => Err(Error {
            source: None,
            error_kind: EntityApiErrorKind::RecordNotFound,
        }),
    }
}

/// Finds a transcription by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds a transcription by meeting recording ID
pub async fn find_by_meeting_recording_id(
    db: &DatabaseConnection,
    meeting_recording_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::transcriptions::Column::MeetingRecordingId.eq(meeting_recording_id))
        .one(db)
        .await?)
}

/// Finds a transcription by AssemblyAI transcript ID
pub async fn find_by_assemblyai_id(
    db: &DatabaseConnection,
    assemblyai_id: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::transcriptions::Column::AssemblyaiTranscriptId.eq(assemblyai_id))
        .one(db)
        .await?)
}

/// Deletes a transcription by ID
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let model = find_by_id(db, id).await?;
    Entity::delete_by_id(model.id).exec(db).await?;
    Ok(())
}
