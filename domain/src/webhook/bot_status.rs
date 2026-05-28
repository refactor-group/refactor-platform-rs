use crate::error::Error;
use crate::meeting_recording::{self as recording_api, MeetingRecordingStatus, RecordingArtifacts};
use events::{DomainEvent, EventPublisher};
use log::*;
use sea_orm::DatabaseConnection;

pub async fn handle(
    db: &DatabaseConnection,
    event_publisher: &EventPublisher,
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
        MeetingRecordingStatus::Completed
            | MeetingRecordingStatus::Failed
            | MeetingRecordingStatus::Cancelled
    ) {
        debug!(
            "bot status: recording {} already terminal ({:?}) — skipping",
            recording.id, recording.status
        );
        return Ok(());
    }

    let now: sea_orm::prelude::DateTimeWithTimeZone = chrono::Utc::now().into();

    let started_at = match status {
        MeetingRecordingStatus::InMeeting | MeetingRecordingStatus::Recording
            if recording.started_at.is_none() =>
        {
            Some(now)
        }
        _ => None,
    };

    let ended_at = match status {
        MeetingRecordingStatus::Processing if recording.ended_at.is_none() => Some(now),
        _ => None,
    };

    recording_api::update_status(
        db,
        recording.id,
        status,
        RecordingArtifacts {
            started_at,
            ended_at,
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
            "bot_status: could not resolve participants for session {}: {:?}",
            coaching_session_id, e
        ),
    }

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use super::*;
    use entity::meeting_recording::Model;
    use entity::Id;
    use events::EventPublisher;
    use sea_orm::{DatabaseBackend, MockDatabase, Transaction, Value};

    fn recording_with_status(status: MeetingRecordingStatus) -> Model {
        let now = chrono::Utc::now();
        Model {
            id: Id::new_v4(),
            coaching_session_id: Id::new_v4(),
            bot_id: "bot-skip-test".to_string(),
            status,
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

    /// Extract a timestamp bind by index from the UPDATE statement in the captured log.
    /// Bind order: status, video_url, audio_url, duration_seconds, started_at (4),
    /// ended_at (5), error_message, updated_at (7), [id in WHERE].
    fn ts_bind_in_update(
        log: &[Transaction],
        bind_index: usize,
    ) -> Option<sea_orm::prelude::DateTimeWithTimeZone> {
        for txn in log {
            for stmt in txn.statements() {
                if stmt.sql.starts_with("UPDATE ") {
                    let binds = &stmt.values.as_ref().expect("update has binds").0;
                    return match &binds[bind_index] {
                        Value::ChronoDateTimeWithTimeZone(opt) => opt.as_deref().copied(),
                        other => panic!("bind[{bind_index}] not a timestamp: {other:?}"),
                    };
                }
            }
        }
        panic!("no UPDATE statement found in transaction log");
    }

    const BIND_STARTED_AT: usize = 4;
    const BIND_ENDED_AT: usize = 5;

    #[tokio::test]
    async fn bot_status_skips_completed_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Completed);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::Joining,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn bot_status_skips_failed_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Failed);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::Recording,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn bot_status_skips_cancelled_recording() {
        let recording = recording_with_status(MeetingRecordingStatus::Cancelled);
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![recording]])
            .into_connection();

        let publisher = EventPublisher::new();
        let result = handle(
            &db,
            &publisher,
            "bot-skip-test",
            MeetingRecordingStatus::InMeeting,
        )
        .await;

        assert!(result.is_ok());
    }

    async fn run_transition_and_capture(
        existing: Model,
        new_status: MeetingRecordingStatus,
    ) -> Vec<Transaction> {
        let after = Model {
            status: new_status.clone(),
            ..existing.clone()
        };
        let db = MockDatabase::new(DatabaseBackend::Postgres)
            .append_query_results(vec![vec![existing.clone()]]) // find_by_bot_id
            .append_query_results(vec![vec![existing]]) // find_by_id inside update_status
            .append_query_results(vec![vec![after]]) // update returns the model
            .into_connection();
        let publisher = EventPublisher::new();
        handle(&db, &publisher, "bot-skip-test", new_status)
            .await
            .expect("handler should succeed");
        db.into_transaction_log()
    }

    #[tokio::test]
    async fn bot_status_writes_started_at_on_first_in_meeting_transition() {
        let before = chrono::Utc::now();
        let existing = recording_with_status(MeetingRecordingStatus::Joining);
        let log = run_transition_and_capture(existing, MeetingRecordingStatus::InMeeting).await;

        let bound = ts_bind_in_update(&log, BIND_STARTED_AT)
            .expect("started_at should be written on first InMeeting transition");
        let after = chrono::Utc::now();
        assert!(
            bound.to_utc() >= before && bound.to_utc() <= after,
            "started_at should be a freshly-captured Utc::now(), got {bound:?}"
        );
    }

    #[tokio::test]
    async fn bot_status_writes_started_at_on_first_recording_transition() {
        let existing = recording_with_status(MeetingRecordingStatus::Joining);
        let log = run_transition_and_capture(existing, MeetingRecordingStatus::Recording).await;

        assert!(
            ts_bind_in_update(&log, BIND_STARTED_AT).is_some(),
            "Recording transition (when no prior started_at) should write started_at"
        );
    }

    #[tokio::test]
    async fn bot_status_preserves_existing_started_at_on_subsequent_active_transition() {
        let original_start: sea_orm::prelude::DateTimeWithTimeZone =
            (chrono::Utc::now() - chrono::Duration::minutes(10)).into();
        let mut existing = recording_with_status(MeetingRecordingStatus::InMeeting);
        existing.started_at = Some(original_start);

        let log = run_transition_and_capture(existing, MeetingRecordingStatus::Recording).await;

        let bound = ts_bind_in_update(&log, BIND_STARTED_AT)
            .expect("started_at should remain non-null after preserve");
        // First-write-wins: the original value, not a fresh Utc::now(), must appear in the bind.
        assert_eq!(
            bound, original_start,
            "subsequent active transition must preserve the original started_at exactly"
        );
    }

    #[tokio::test]
    async fn bot_status_writes_ended_at_on_processing_transition() {
        let original_start: sea_orm::prelude::DateTimeWithTimeZone =
            (chrono::Utc::now() - chrono::Duration::minutes(30)).into();
        let mut existing = recording_with_status(MeetingRecordingStatus::Recording);
        existing.started_at = Some(original_start);

        let before = chrono::Utc::now();
        let log = run_transition_and_capture(existing, MeetingRecordingStatus::Processing).await;

        // started_at preserved exactly.
        assert_eq!(
            ts_bind_in_update(&log, BIND_STARTED_AT),
            Some(original_start),
            "Processing transition must preserve original started_at"
        );
        // ended_at freshly written.
        let ended = ts_bind_in_update(&log, BIND_ENDED_AT)
            .expect("Processing transition should write ended_at");
        let after = chrono::Utc::now();
        assert!(
            ended.to_utc() >= before && ended.to_utc() <= after,
            "ended_at should be a freshly-captured Utc::now(), got {ended:?}"
        );
    }

    #[tokio::test]
    async fn bot_status_does_not_write_started_at_on_non_active_transition() {
        let existing = recording_with_status(MeetingRecordingStatus::Pending);
        let log = run_transition_and_capture(existing, MeetingRecordingStatus::Joining).await;

        assert_eq!(
            ts_bind_in_update(&log, BIND_STARTED_AT),
            None,
            "Joining transition should not write started_at"
        );
        assert_eq!(
            ts_bind_in_update(&log, BIND_ENDED_AT),
            None,
            "Joining transition should not write ended_at"
        );
    }
}
