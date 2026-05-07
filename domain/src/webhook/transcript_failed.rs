use crate::error::Error;
use crate::transcription::{self as transcription_api, TranscriptionStatus};
use events::{DomainEvent, EventPublisher};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
    transcript_id: &str,
    error_message: Option<String>,
) -> Result<(), Error> {
    let transcription = match transcription_api::find_by_external_id(db, transcript_id).await? {
        Some(t) => t,
        None => {
            warn!(
                "transcript.failed: no transcription for external_id={}",
                transcript_id
            );
            return Ok(());
        }
    };

    transcription_api::update_status(
        db,
        transcription.id,
        TranscriptionStatus::Failed,
        None,
        None,
        error_message,
    )
    .await?;

    let coaching_session_id = transcription.coaching_session_id;
    match crate::coaching_session::find_participant_ids(db, coaching_session_id).await {
        Ok(user_ids) => {
            event_publisher
                .publish(DomainEvent::TranscriptionUpdated {
                    coaching_session_id,
                    notify_user_ids: user_ids,
                })
                .await;
        }
        Err(e) => warn!(
            "transcript_failed: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }

    Ok(())
}
