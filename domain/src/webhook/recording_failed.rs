use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus, RecordingArtifacts};
use events::EventPublisher;
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    bot_id: &str,
    error_message: Option<String>,
) -> Result<(), Error> {
    let recording = match recording_api::find_by_bot_id(db, bot_id).await? {
        Some(r) => r,
        None => {
            warn!("recording.failed: no recording for bot_id={}", bot_id);
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
            "recording.failed: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    let ended_at = recording
        .ended_at
        .is_none()
        .then(|| chrono::Utc::now().into());

    recording_api::update_status(
        db,
        recording.id,
        MeetingRecordingStatus::Failed,
        RecordingArtifacts {
            error_message,
            ended_at,
            ..Default::default()
        },
    )
    .await?;

    let coaching_session_id = recording.coaching_session_id;
    recording_api::publish_updated(db, event_publisher, coaching_session_id).await;

    Ok(())
}
