use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
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

    recording_api::update_status(
        db,
        recording.id,
        MeetingRecordingStatus::Failed,
        None,
        None,
        None,
        None,
        None,
        error_message,
    )
    .await?;

    Ok(())
}
