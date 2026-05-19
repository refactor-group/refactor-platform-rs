use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus, RecordingArtifacts};
use events::{DomainEvent, EventPublisher};
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
            warn!("bot.fatal: no recording for bot_id={}", bot_id);
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
            "bot.fatal: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    recording_api::update_status(
        db,
        recording.id,
        MeetingRecordingStatus::Failed,
        RecordingArtifacts {
            error_message,
            ..Default::default()
        },
    )
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
            "bot_fatal: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }

    Ok(())
}
