use entity::transcript_participant::{ActiveModel, Column, Entity, Model};
use entity::Id;
use log::debug;
use sea_orm::{entity::prelude::*, ConnectionTrait};

use super::error::Error;

/// Inserts participants in one statement; an empty batch is a no-op.
pub async fn create_batch<C: ConnectionTrait>(
    db: &C,
    participants: Vec<ActiveModel>,
) -> Result<Vec<Model>, Error> {
    debug!("Inserting {} transcript participants", participants.len());

    if participants.is_empty() {
        return Ok(vec![]);
    }

    Ok(Entity::insert_many(participants)
        .exec_with_returning(db)
        .await?)
}

/// Every participant recorded for a transcription, in no particular order.
pub async fn find_by_transcription<C: ConnectionTrait>(
    db: &C,
    transcription_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TranscriptionId.eq(transcription_id))
        .all(db)
        .await?)
}
