use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus};
use crate::transcription as transcription_api;
use entity::Id;
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;
use std::sync::Arc;

pub async fn handle(
    db: Arc<DatabaseConnection>,
    config: Config,
    bot_id: &str,
    recall_recording_id: &str,
    coaching_session_id: Option<Id>,
) -> Result<(), Error> {
    let coaching_session_id = match coaching_session_id {
        Some(id) => id,
        None => {
            warn!("recording.done: missing/invalid coaching_session_id in bot metadata — skipping");
            return Ok(());
        }
    };

    let recording = match recording_api::find_by_bot_id(&db, bot_id).await? {
        Some(r) => r,
        None => {
            warn!("recording.done: no recording for bot_id={}", bot_id);
            return Ok(());
        }
    };

    // Idempotency: skip if a transcription already exists for this session.
    if transcription_api::find_by_coaching_session(&db, coaching_session_id)
        .await?
        .is_some()
    {
        warn!(
            "recording.done: transcription already exists for session={} — skipping",
            coaching_session_id
        );
        return Ok(());
    }

    recording_api::update_status(
        &db,
        recording.id,
        MeetingRecordingStatus::Completed,
        None,
        None,
        None,
        None,
        Some(chrono::Utc::now().into()),
        None,
    )
    .await?;

    let recall_recording_id = recall_recording_id.to_string();

    tokio::spawn(async move {
        if let Err(e) =
            crate::transcription::start(&db, &config, &recording, &recall_recording_id).await
        {
            error!(
                "recording.done: transcription start failed for session={}: {:?}",
                coaching_session_id, e
            );
            let _ = recording_api::update_status(
                &db,
                recording.id,
                MeetingRecordingStatus::Failed,
                None,
                None,
                None,
                None,
                None,
                Some(e.to_string()),
            )
            .await;
        }
    });

    Ok(())
}
