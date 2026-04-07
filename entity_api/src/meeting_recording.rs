use super::error::{EntityApiErrorKind, Error};
use entity::meeting_recording::{ActiveModel, Column, Entity, MeetingRecordingStatus, Model};
use entity::Id;
use log::debug;
use sea_orm::{
    entity::prelude::*,
    ActiveValue::{Set, Unchanged},
    DatabaseConnection, Order, QueryOrder, TryIntoModel,
};

/// Creates a new meeting recording record
pub async fn create(db: &DatabaseConnection, model: Model) -> Result<Model, Error> {
    debug!(
        "Creating meeting recording for coaching_session_id: {}",
        model.coaching_session_id
    );

    let now = chrono::Utc::now();

    let active_model = ActiveModel {
        coaching_session_id: Set(model.coaching_session_id),
        bot_id: Set(model.bot_id),
        status: Set(model.status),
        video_url: Set(model.video_url),
        audio_url: Set(model.audio_url),
        duration_seconds: Set(model.duration_seconds),
        started_at: Set(model.started_at),
        ended_at: Set(model.ended_at),
        error_message: Set(model.error_message),
        created_at: Set(now.into()),
        updated_at: Set(now.into()),
        ..Default::default()
    };

    Ok(active_model.save(db).await?.try_into_model()?)
}

/// Returns the most recent recording for a coaching session (by `created_at DESC`)
pub async fn find_latest_by_coaching_session(
    db: &DatabaseConnection,
    session_id: Id,
) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::CoachingSessionId.eq(session_id))
        .order_by(Column::CreatedAt, Order::Desc)
        .one(db)
        .await?)
}

/// Finds a recording by Recall.ai bot ID — used by webhook handlers
pub async fn find_by_bot_id(db: &DatabaseConnection, bot_id: &str) -> Result<Option<Model>, Error> {
    Ok(Entity::find()
        .filter(Column::BotId.eq(bot_id))
        .one(db)
        .await?)
}

/// Updates recording status and optional artifact fields
#[allow(clippy::too_many_arguments)]
pub async fn update_status(
    db: &DatabaseConnection,
    id: Id,
    status: MeetingRecordingStatus,
    video_url: Option<String>,
    audio_url: Option<String>,
    duration_seconds: Option<i32>,
    started_at: Option<DateTimeWithTimeZone>,
    ended_at: Option<DateTimeWithTimeZone>,
    error_message: Option<String>,
) -> Result<Model, Error> {
    let existing = Entity::find_by_id(id).one(db).await?.ok_or(Error {
        source: None,
        error_kind: EntityApiErrorKind::RecordNotFound,
    })?;

    debug!("Updating meeting recording status: {id}");

    let active_model = ActiveModel {
        id: Unchanged(existing.id),
        coaching_session_id: Unchanged(existing.coaching_session_id),
        bot_id: Unchanged(existing.bot_id),
        status: Set(status),
        video_url: Set(video_url.or(existing.video_url)),
        audio_url: Set(audio_url.or(existing.audio_url)),
        duration_seconds: Set(duration_seconds.or(existing.duration_seconds)),
        started_at: Set(started_at.or(existing.started_at)),
        ended_at: Set(ended_at.or(existing.ended_at)),
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
            bot_id: "recall-bot-abc123".to_string(),
            status: MeetingRecordingStatus::Pending,
            video_url: None,
            audio_url: None,
            duration_seconds: None,
            started_at: None,
            ended_at: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn create_returns_a_new_meeting_recording() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = create(&db, model.clone()).await?;

        assert_eq!(result.coaching_session_id, model.coaching_session_id);
        assert_eq!(result.bot_id, model.bot_id);
        assert_eq!(result.status, MeetingRecordingStatus::Pending);

        Ok(())
    }

    #[tokio::test]
    async fn find_latest_by_coaching_session_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_latest_by_coaching_session(&db, Id::new_v4()).await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_latest_by_coaching_session_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_latest_by_coaching_session(&db, model.coaching_session_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().bot_id, model.bot_id);
        Ok(())
    }

    #[tokio::test]
    async fn find_by_bot_id_returns_none_when_not_found() -> Result<(), Error> {
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results::<Model, Vec<Model>, _>(vec![vec![]])
            .into_connection();

        let result = find_by_bot_id(&db, "nonexistent-bot").await?;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn find_by_bot_id_returns_model_when_found() -> Result<(), Error> {
        let model = test_model();

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .into_connection();

        let result = find_by_bot_id(&db, &model.bot_id).await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().bot_id, model.bot_id);
        Ok(())
    }

    #[tokio::test]
    async fn update_status_updates_recording_status() -> Result<(), Error> {
        let model = test_model();
        let mut updated = model.clone();
        updated.status = MeetingRecordingStatus::Recording;

        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![model.clone()]])
            .append_query_results(vec![vec![updated.clone()]])
            .into_connection();

        let result = update_status(
            &db,
            model.id,
            MeetingRecordingStatus::Recording,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await?;

        assert_eq!(result.status, MeetingRecordingStatus::Recording);
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
            MeetingRecordingStatus::Failed,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        assert!(result.is_err());
    }
}
