//! CRUD operations for meeting_recordings table.

use super::error::{EntityApiErrorKind, Error};
use entity::meeting_recording_status::MeetingRecordingStatus;
use entity::meeting_recordings::{ActiveModel, Entity, Model};
use entity::Id;
use log::*;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, QueryOrder, TryIntoModel,
};

/// Creates a new meeting recording record
pub async fn create(db: &DatabaseConnection, coaching_session_id: Id) -> Result<Model, Error> {
    debug!("Creating new meeting recording for session: {coaching_session_id}");

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        coaching_session_id: Set(coaching_session_id),
        status: Set(MeetingRecordingStatus::Pending),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Updates an existing meeting recording record
pub async fn update(db: &DatabaseConnection, id: Id, model: Model) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating meeting recording: {id}");

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                coaching_session_id: Unchanged(existing.coaching_session_id),
                recall_bot_id: Set(model.recall_bot_id),
                status: Set(model.status),
                recording_url: Set(model.recording_url),
                duration_seconds: Set(model.duration_seconds),
                started_at: Set(model.started_at),
                ended_at: Set(model.ended_at),
                error_message: Set(model.error_message),
                created_at: Unchanged(existing.created_at),
                updated_at: Set(chrono::Utc::now().into()),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            debug!("Meeting recording with id {id} not found");
            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

/// Updates just the status of a meeting recording
pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: MeetingRecordingStatus,
    error_message: Option<String>,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Updating meeting recording status to {:?}: {id}", status);

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                coaching_session_id: Unchanged(existing.coaching_session_id),
                recall_bot_id: Unchanged(existing.recall_bot_id),
                status: Set(status),
                recording_url: Unchanged(existing.recording_url),
                duration_seconds: Unchanged(existing.duration_seconds),
                started_at: Unchanged(existing.started_at),
                ended_at: Unchanged(existing.ended_at),
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

/// Finds a meeting recording by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds a meeting recording by coaching session ID
pub async fn find_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::meeting_recordings::Column::CoachingSessionId.eq(coaching_session_id))
        .one(db)
        .await?)
}

/// Finds the latest meeting recording for a coaching session
pub async fn find_latest_by_coaching_session_id(
    db: &DatabaseConnection,
    coaching_session_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::meeting_recordings::Column::CoachingSessionId.eq(coaching_session_id))
        .order_by_desc(entity::meeting_recordings::Column::CreatedAt)
        .one(db)
        .await?)
}

/// Finds a meeting recording by Recall.ai bot ID
pub async fn find_by_recall_bot_id(
    db: &DatabaseConnection,
    recall_bot_id: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::meeting_recordings::Column::RecallBotId.eq(recall_bot_id))
        .one(db)
        .await?)
}

/// Deletes a meeting recording by ID
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let model = find_by_id(db, id).await?;
    Entity::delete_by_id(model.id).exec(db).await?;
    Ok(())
}
