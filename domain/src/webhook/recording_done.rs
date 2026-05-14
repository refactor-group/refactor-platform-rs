use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus};
use entity::Id;
use events::{DomainEvent, EventPublisher};
use log::*;
use meeting_ai::traits::transcription as transcription_trait;
use sea_orm::DatabaseConnection;
use std::sync::Arc;

pub async fn handle(
    db: Arc<DatabaseConnection>,
    transcription_provider: Option<Arc<dyn transcription_trait::Provider>>,
    event_publisher: EventPublisher,
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

    // Reject if the session in bot metadata doesn't match what we stored — prevents
    // a tampered payload from triggering events or transcription under a different session.
    if recording.coaching_session_id != coaching_session_id {
        warn!(
            "recording.done: coaching_session_id mismatch for bot_id={} — \
             metadata claims {} but recording belongs to {} — rejecting",
            bot_id, coaching_session_id, recording.coaching_session_id
        );
        return Ok(());
    }

    // Atomically claim this recording as Completed. Returns false if the recording is
    // already terminal (Completed, Failed, or Cancelled — including the user-cancelled
    // case). This prevents concurrent recording.done webhooks from both reaching
    // create_transcription (double billing).
    if !recording_api::try_claim_completed(&db, recording.id).await? {
        debug!(
            "recording.done: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    match crate::coaching_session::find_participant_ids(&db, coaching_session_id).await {
        Ok(user_ids) => {
            event_publisher
                .publish(DomainEvent::MeetingRecordingUpdated {
                    coaching_session_id,
                    notify_user_ids: user_ids,
                })
                .await;
        }
        Err(e) => warn!(
            "recording_done: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }

    let recall_recording_id = recall_recording_id.to_string();

    tokio::spawn(async move {
        match crate::transcription::start(
            &db,
            transcription_provider.as_deref(),
            &recording,
            &recall_recording_id,
        )
        .await
        {
            Ok(_) => {
                match crate::coaching_session::find_participant_ids(&db, coaching_session_id).await
                {
                    Ok(user_ids) => {
                        event_publisher
                            .publish(DomainEvent::TranscriptionUpdated {
                                coaching_session_id,
                                notify_user_ids: user_ids,
                            })
                            .await;
                    }
                    Err(e) => warn!(
                        "recording_done: could not resolve participants for TranscriptionUpdated: {:?}",
                        e
                    ),
                }
            }
            Err(e) => {
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
                match crate::coaching_session::find_participant_ids(&db, coaching_session_id).await
                {
                    Ok(user_ids) => {
                        event_publisher
                            .publish(DomainEvent::MeetingRecordingUpdated {
                                coaching_session_id,
                                notify_user_ids: user_ids,
                            })
                            .await;
                    }
                    Err(e) => warn!(
                        "recording_done: could not resolve participants for failure MeetingRecordingUpdated: {:?}",
                        e
                    ),
                }
            }
        }
    });

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::meeting_recording::{MeetingRecordingStatus, Model as RecordingModel};
    use entity::Id;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
    use std::sync::Arc;

    fn recording_for_session(session_id: Id) -> RecordingModel {
        let now = chrono::Utc::now();
        RecordingModel {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            bot_id: "bot-rd-test".to_string(),
            status: MeetingRecordingStatus::Processing,
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
    async fn recording_done_skips_when_try_claim_completed_returns_false() {
        let session_id = Id::new_v4();
        let recording = recording_for_session(session_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_bot_id
                .append_query_results(vec![vec![recording]])
                // try_claim_completed — 0 rows affected means already terminal
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }])
                .into_connection(),
        );

        let publisher = EventPublisher::new();
        let result = handle(
            Arc::clone(&db),
            None,
            publisher,
            "bot-rd-test",
            "rec-123",
            Some(session_id),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn recording_done_skips_when_coaching_session_id_is_none() {
        let db = Arc::new(MockDatabase::new(DatabaseBackend::Postgres).into_connection());

        let publisher = EventPublisher::new();
        let result = handle(
            Arc::clone(&db),
            None,
            publisher,
            "bot-any",
            "rec-any",
            None, // missing session_id — handler logs and returns Ok
        )
        .await;

        assert!(result.is_ok());
    }
}
