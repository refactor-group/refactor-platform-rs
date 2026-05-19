use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus, RecordingArtifacts};
use events::{DomainEvent, EventPublisher};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    bot_id: &str,
    status: MeetingRecordingStatus,
) -> Result<(), Error> {
    let recording = match recording_api::find_by_bot_id(db, bot_id).await? {
        Some(r) => r,
        None => {
            warn!("bot status: no recording for bot_id={}", bot_id);
            return Ok(());
        }
    };

    if matches!(
        recording.status,
        MeetingRecordingStatus::Completed
            | MeetingRecordingStatus::Failed
            | MeetingRecordingStatus::Cancelled
    ) {
        debug!(
            "bot status: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    recording_api::update_status(db, recording.id, status, RecordingArtifacts::default())
        .await?;

    let coaching_session_id = recording.coaching_session_id;
    match crate::coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(user_ids) => {
            event_publisher
                .publish(DomainEvent::MeetingRecordingUpdated {
                    coaching_session_id,
                    notify_user_ids: user_ids,
                })
                .await;
        }
        Err(e) => warn!(
            "bot_status: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::meeting_recording::Model;
    use entity::Id;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase};

    fn recording_with_status(status: MeetingRecordingStatus) -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            bot_id: "bot-skip-test".to_string(),
            status,
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
    async fn bot_status_skips_completed_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Completed);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::Joining,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn bot_status_skips_failed_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Failed);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::Recording,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn bot_status_skips_cancelled_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Cancelled);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::InMeeting,
        )
        .await;

        assert!(result.is_ok());
    }
}
