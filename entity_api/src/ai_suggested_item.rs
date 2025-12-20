//! CRUD operations for ai_suggested_items table.

use super::error::{EntityApiErrorKind, Error};
use entity::ai_suggested_items::{ActiveModel, Entity, Model};
use entity::ai_suggestion::{AiSuggestionStatus, AiSuggestionType};
use entity::Id;
use log::*;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

/// Creates a new AI suggested item
pub async fn create(
    db: &DatabaseConnection,
    transcription_id: Id,
    item_type: AiSuggestionType,
    content: String,
    source_text: Option<String>,
    confidence: Option<f64>,
) -> Result<Model, Error> {
    debug!("Creating new AI suggestion for transcription: {transcription_id}");

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        transcription_id: Set(transcription_id),
        item_type: Set(item_type),
        content: Set(content),
        source_text: Set(source_text),
        confidence: Set(confidence),
        status: Set(AiSuggestionStatus::Pending),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Accepts an AI suggested item, linking it to the created entity
pub async fn accept(
    db: &DatabaseConnection,
    id: Id,
    accepted_entity_id: Id,
) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Accepting AI suggestion: {id}");

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                transcription_id: Unchanged(existing.transcription_id),
                item_type: Unchanged(existing.item_type),
                content: Unchanged(existing.content),
                source_text: Unchanged(existing.source_text),
                confidence: Unchanged(existing.confidence),
                status: Set(AiSuggestionStatus::Accepted),
                accepted_entity_id: Set(Some(accepted_entity_id)),
                created_at: Unchanged(existing.created_at),
                updated_at: Set(chrono::Utc::now().into()),
            };

            Ok(active_model.update(db).await?.try_into_model()?)
        }
        None => {
            debug!("AI suggestion with id {id} not found");
            Err(Error {
                source: None,
                error_kind: EntityApiErrorKind::RecordNotFound,
            })
        }
    }
}

/// Dismisses an AI suggested item
pub async fn dismiss(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    let result = Entity::find_by_id(id).one(db).await?;

    match result {
        Some(existing) => {
            debug!("Dismissing AI suggestion: {id}");

            let active_model = ActiveModel {
                id: Unchanged(existing.id),
                transcription_id: Unchanged(existing.transcription_id),
                item_type: Unchanged(existing.item_type),
                content: Unchanged(existing.content),
                source_text: Unchanged(existing.source_text),
                confidence: Unchanged(existing.confidence),
                status: Set(AiSuggestionStatus::Dismissed),
                accepted_entity_id: Unchanged(existing.accepted_entity_id),
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

/// Finds an AI suggested item by ID
pub async fn find_by_id(db: &DatabaseConnection, id: Id) -> Result<Model, Error> {
    Entity::find_by_id(id).one(db).await?.ok_or_else(|| Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })
}

/// Finds all AI suggestions for a transcription
pub async fn find_by_transcription_id(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::ai_suggested_items::Column::TranscriptionId.eq(transcription_id))
        .all(db)
        .await?)
}

/// Finds pending AI suggestions for a transcription
pub async fn find_pending_by_transcription_id(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::ai_suggested_items::Column::TranscriptionId.eq(transcription_id))
        .filter(entity::ai_suggested_items::Column::Status.eq(AiSuggestionStatus::Pending))
        .all(db)
        .await?)
}

/// Finds AI suggestions by type for a transcription
pub async fn find_by_type(
    db: &DatabaseConnection,
    transcription_id: Id,
    item_type: AiSuggestionType,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(entity::ai_suggested_items::Column::TranscriptionId.eq(transcription_id))
        .filter(entity::ai_suggested_items::Column::ItemType.eq(item_type))
        .all(db)
        .await?)
}

/// Deletes an AI suggested item by ID
pub async fn delete_by_id(db: &DatabaseConnection, id: Id) -> Result<(), Error> {
    let model = find_by_id(db, id).await?;
    Entity::delete_by_id(model.id).exec(db).await?;
    Ok(())
}
