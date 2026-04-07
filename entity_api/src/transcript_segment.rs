use super::error::Error;
use entity::transcript_segment::{ActiveModel, Column, Entity, Model};
use entity::Id;
use log::debug;
use sea_orm::{entity::prelude::*, DatabaseConnection, Order, QueryOrder};

/// Inserts multiple transcript segments in a single operation
pub async fn create_batch(
    db: &DatabaseConnection,
    segments: Vec<ActiveModel>,
) -> Result<Vec<Model>, Error> {
    debug!("Inserting {} transcript segments", segments.len());

    Ok(Entity::insert_many(segments)
        .exec_with_returning_many(db)
        .await?)
}

/// Returns all segments for a transcription ordered by start time
pub async fn find_by_transcription(
    db: &DatabaseConnection,
    transcription_id: Id,
) -> Result<Vec<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::TranscriptionId.eq(transcription_id))
        .order_by(Column::StartMs, Order::Asc)
        .all(db)
        .await?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_model(transcription_id: Id) -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            transcription_id,
            speaker_label: "Jane Smith".to_string(),
            text: "What goals are you working toward this quarter?".to_string(),
            start_ms: 1000,
            end_ms: 5200,
            confidence: Some(0.97),
            sentiment: Some("neutral".to_string()),
            created_at: now.into(),
        }
    }

    #[tokio::test]
    async fn find_by_transcription_returns_empty_when_none_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_transcription(&db, Id::new_v4()).await?;
        assert!(result.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_transcription_returns_segments_when_found() -> Result<(), Error> {
        let transcription_id = Id::new_v4();
        let model = test_model(transcription_id);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_transcription(&db, transcription_id).await?;
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].speaker_label, "Jane Smith");
        assert_eq!(result[0].start_ms, 1000);
        Ok(())
    }
}
