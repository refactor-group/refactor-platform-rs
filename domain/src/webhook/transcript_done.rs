use crate::error::Error;
use crate::transcription::{self as transcription_api, TranscriptionStatus};
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;
use std::sync::Arc;

pub async fn handle(
    db: Arc<DatabaseConnection>,
    config: Config,
    transcript_id: &str,
) -> Result<(), Error> {
    let transcription = match transcription_api::find_by_external_id(&db, transcript_id).await? {
        Some(t) => t,
        None => {
            // Retry — transcript.processing should have created this record already.
            return Err(Error {
                source: None,
                error_kind: crate::error::DomainErrorKind::Internal(
                    crate::error::InternalErrorKind::Entity(
                        crate::error::EntityErrorKind::NotFound,
                    ),
                ),
            });
        }
    };

    // Atomic claim: Queued → Processing. Rows-affected = 0 means already claimed or terminal.
    match transcription_api::try_claim_for_processing(&db, transcription.id).await? {
        true => {}
        false => {
            debug!(
                "transcript.done: transcription {} not in queued state — skipping",
                transcription.id
            );
            return Ok(());
        }
    }

    let transcription_id = transcription.id;
    let transcript_id = transcript_id.to_string();

    tokio::spawn(async move {
        if let Err(e) = crate::transcription::handle_completion(&db, &config, &transcript_id).await
        {
            error!(
                "transcript.done: completion failed for external_id={}: {:?}",
                transcript_id, e
            );
            let _ = transcription_api::update_status(
                &db,
                transcription_id,
                TranscriptionStatus::Failed,
                None,
                None,
                Some(e.to_string()),
            )
            .await;
        }
    });

    Ok(())
}
