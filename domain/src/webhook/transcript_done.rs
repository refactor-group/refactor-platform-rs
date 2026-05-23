use crate::error::Error;
use crate::transcription::{self as transcription_api, TranscriptionStatus};
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
    transcript_id: &str,
) -> Result<(), Error> {
    let transcription = match transcription_api::find_by_external_id(&db, transcript_id).await? {
        Some(t) => t,
        None => {
            // No transcription row means recording.done was never processed or
            // transcription::start failed — a permanent condition. Return Ok so
            // Svix does not retry for ~27 hours on a miss that will never resolve.
            warn!(
                "transcript.done: no transcription found for external_id={} — skipping",
                transcript_id
            );
            return Ok(());
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
    let coaching_session_id: Id = transcription.coaching_session_id;
    let transcript_id = transcript_id.to_string();

    tokio::spawn(async move {
        let result = crate::transcription::handle_completion(
            &db,
            transcription_provider.as_deref(),
            &transcript_id,
        )
        .await;

        if let Err(e) = result {
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

        match crate::coaching_session::find_participant_ids(&db, coaching_session_id).await {
            Ok(user_ids) => {
                event_publisher
                    .publish(DomainEvent::TranscriptionUpdated {
                        coaching_session_id,
                        notify_user_ids: user_ids,
                    })
                    .await;
            }
            Err(e) => warn!(
                "transcript_done: could not resolve participants for session {}: {:?}",
                coaching_session_id, e
            ),
        }
    });

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::transcription::{Model as TranscriptionModel, TranscriptionStatus};
    use entity::Id;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase, MockExecResult};
    use std::sync::Arc;

    fn queued_transcription() -> TranscriptionModel {
        let now = chrono::Utc::now();
        TranscriptionModel {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            meeting_recording_id: Id::new_v4(),
            external_id: "ext-td-test".to_string(),
            recall_recording_id: None,
            status: TranscriptionStatus::Queued,
            language_code: None,
            speaker_count: None,
            word_count: None,
            duration_seconds: None,
            confidence: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    #[tokio::test]
    async fn transcript_done_skips_when_try_claim_for_processing_returns_false() {
        let transcription = queued_transcription();

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // find_by_external_id
                .append_query_results(vec![vec![transcription]])
                // try_claim_for_processing — 0 rows affected means already claimed
                .append_exec_results(vec![MockExecResult {
                    last_insert_id: 0,
                    rows_affected: 0,
                }])
                .into_connection(),
        );

        let publisher = EventPublisher::new();
        let result = handle(Arc::clone(&db), None, publisher, "ext-td-test").await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn transcript_done_skips_when_external_id_not_found() {
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results::<TranscriptionModel, Vec<TranscriptionModel>, _>(vec![
                    vec![],
                ])
                .into_connection(),
        );

        let publisher = EventPublisher::new();
        let result = handle(Arc::clone(&db), None, publisher, "nonexistent-id").await;

        assert!(result.is_ok());
    }
}
