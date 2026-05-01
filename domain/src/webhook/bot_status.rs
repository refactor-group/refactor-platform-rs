use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
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
        MeetingRecordingStatus::Completed | MeetingRecordingStatus::Failed
    ) {
        debug!(
            "bot status: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    recording_api::update_status(db, recording.id, status, None, None, None, None, None, None)
        .await?;

    Ok(())
}
