use super::error::{EntityApiErrorKind, Error};
use entity::transcription::{ActiveModel, Column, Entity, Model, TranscriptionStatus};
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, TryIntoModel,
};

/// Creates a new transcription record
pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    debug!(
        "Creating transcription for coaching_session_id: {}, external_id: {}",
        model.coaching_session_id, model.external_id
    );

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        coaching_session_id: Set(model.coaching_session_id),
        meeting_recording_id: Set(model.meeting_recording_id),
        external_id: Set(model.external_id),
        recall_recording_id: Set(model.recall_recording_id),
        status: Set(model.status),
        language_code: Set(model.language_code),
        speaker_count: Set(model.speaker_count),
        word_count: Set(model.word_count),
        duration_seconds: Set(model.duration_seconds),
        confidence: Set(model.confidence),
        error_message: Set(model.error_message),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Returns the transcription for a coaching session
pub async fn find_by_coaching_session(
    db: &DatabaseConnection,
    session_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(session_id))
        .one(db)
        .await?)
}

/// Finds a transcription by Recall.ai transcript ID — used by webhook handlers
pub async fn find_by_external_id(
    db: &DatabaseConnection,
    external_id: &str,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::ExternalId.eq(external_id))
        .one(db)
        .await?)
}

/// Updates transcription status and optional metadata fields
pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: TranscriptionStatus,
    word_count: Option<i32>,
    confidence: Option<f64>,
    error_message: Option<String>,
) -> Result<Model, Error> {
    let existing = Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    debug!("Updating transcription status: {id}");

    let active_model = ActiveModel {
        id: Unchanged(existing.id),
        coaching_session_id: Unchanged(existing.coaching_session_id),
        meeting_recording_id: Unchanged(existing.meeting_recording_id),
        external_id: Unchanged(existing.external_id),
        recall_recording_id: Unchanged(existing.recall_recording_id),
        status: Set(status),
        language_code: Unchanged(existing.language_code),
        speaker_count: Unchanged(existing.speaker_count),
        word_count: Set(word_count.or(existing.word_count)),
        duration_seconds: Unchanged(existing.duration_seconds),
        confidence: Set(confidence.or(existing.confidence)),
        error_message: Set(error_message.or(existing.error_message)),
        created_at: Unchanged(existing.created_at),
        updated_at: Set(chrono::Utc::now().into()),
    };

    Ok(active_model.update(db).await?.try_into_model()?)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn test_model() -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            meeting_recording_id: Id::new_v4(),
            external_id: "recall-transcript-abc123".to_string(),
            recall_recording_id: Some("recall-recording-abc123".to_string()),
            status: TranscriptionStatus::Queued,
            language_code: None,
            speaker_count: None,
            word_count: None,
            duration_seconds: None,
            confidence: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_transcription() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = create(&db, model.clone()).await?;

        assert_eq!(result.coaching_session_id, model.coaching_session_id);
        assert_eq!(result.external_id, model.external_id);
        assert_eq!(result.status, TranscriptionStatus::Queued);

        Ok(())
    }

    #[tokio::test]
    async fn find_by_coaching_session_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_coaching_session(&db, Id::new_v4()).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_coaching_session_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_coaching_session(&db, model.coaching_session_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().external_id, model.external_id);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_external_id_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_external_id(&db, "nonexistent-id").await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_external_id_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_external_id(&db, &model.external_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().external_id, model.external_id);
        Ok(())
    }

    #[tokio::test]
    async fn update_status_updates_transcription_status() -> Result<(), Error> {
        let model = test_model();
        let mut updated = model.clone();
        updated.status = TranscriptionStatus::Completed;
        updated.word_count = Some(4200);
        updated.confidence = Some(0.94);

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .append_query_results(vec![vec![updated.clone()]])
            .into_connection();

        let result = update_status(
            &db,
            model.id,
            TranscriptionStatus::Completed,
            Some(4200),
            Some(0.94),
            None,
        )
        .await?;

        assert_eq!(result.status, TranscriptionStatus::Completed);
        assert_eq!(result.word_count, Some(4200));
        assert_eq!(result.confidence, Some(0.94));
        Ok(())
    }

    #[tokio::test]
    async fn update_status_returns_error_when_not_found() {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = update_status(
            &db,
            Id::new_v4(),
            TranscriptionStatus::Failed,
            None,
            None,
            Some("error".to_string()),
        )
        .await;

        assert!(result.is_err());
    }
}
