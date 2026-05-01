use crate::error::Error;
use crate::transcription::{self as transcription_api, TranscriptionStatus};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
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

    Ok(())
}
