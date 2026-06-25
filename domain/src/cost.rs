use crate::cost_metric::Metric;
use crate::error::Error;
use crate::pipeline_provider::Provider;
use crate::Id;
use entity::platform_cost_metrics::Model as CostMetricsModel;
use entity_api::{cost_pricing_config, meeting_recording as recording_api, platform_cost_metrics};
use log::warn;
use sea_orm::DatabaseConnection;

/// Records the Recall.ai bot-minutes cost for a completed recording.
///
/// Fetches the recording fresh from the DB so `duration_seconds` reflects the
/// value written by `try_claim_completed`. No-ops with a warning if no pricing
/// row is configured.
pub async fn record_bot_minutes(db: &DatabaseConnection, recording_id: Id) -> Result<(), Error> {
    let rate =
        match cost_pricing_config::find_current_rate(db, Provider::RecallAi, Metric::BotMinutes)
            .await?
        {
            Some(r) => r,
            None => {
                warn!(
                    "cost: no pricing config for (RecallAi, BotMinutes) — skipping recording {}",
                    recording_id
                );
                return Ok(());
            }
        };

    let recording = match recording_api::find_by_id(db, recording_id).await? {
        Some(r) => r,
        None => {
            warn!(
                "cost: recording {} not found — skipping bot minutes cost",
                recording_id
            );
            return Ok(());
        }
    };

    let duration_seconds = match recording.duration_seconds {
        Some(d) => d,
        None => {
            warn!(
                "cost: recording {} has no duration_seconds — skipping bot minutes cost",
                recording_id
            );
            return Ok(());
        }
    };

    let quantity = duration_seconds as f64 / 60.0;

    platform_cost_metrics::create(
        db,
        CostMetricsModel {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric: Metric::BotMinutes,
            coaching_session_id: Some(recording.coaching_session_id),
            source_record_id: recording_id,
            cost_low: quantity * rate.cost_per_unit_low,
            cost_high: quantity * rate.cost_per_unit_high,
            cost_avg: quantity * rate.cost_per_unit_avg,
            created_at: chrono::Utc::now().fixed_offset(),
        },
    )
    .await?;

    Ok(())
}

/// Records the Recall.ai transcription-hours cost for a completed transcription.
///
/// Fetches the parent recording to derive duration. No-ops with a warning if no
/// pricing row is configured or the recording cannot be found.
pub async fn record_transcription_hours(
    db: &DatabaseConnection,
    transcription_id: Id,
    meeting_recording_id: Id,
) -> Result<(), Error> {
    let rate = match cost_pricing_config::find_current_rate(
        db,
        Provider::RecallAi,
        Metric::TranscriptionHours,
    )
    .await?
    {
        Some(r) => r,
        None => {
            warn!(
                "cost: no pricing config for (RecallAi, TranscriptionHours) — skipping transcription {}",
                transcription_id
            );
            return Ok(());
        }
    };

    let recording = match recording_api::find_by_id(db, meeting_recording_id).await? {
        Some(r) => r,
        None => {
            warn!(
                "cost: recording {} not found — skipping transcription hours cost",
                meeting_recording_id
            );
            return Ok(());
        }
    };

    let duration_seconds = match recording.duration_seconds {
        Some(d) => d,
        None => {
            warn!(
                "cost: recording {} has no duration_seconds — skipping transcription hours cost for transcription {}",
                meeting_recording_id, transcription_id
            );
            return Ok(());
        }
    };

    let quantity = duration_seconds as f64 / 3600.0;

    platform_cost_metrics::create(
        db,
        CostMetricsModel {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric: Metric::TranscriptionHours,
            coaching_session_id: Some(recording.coaching_session_id),
            source_record_id: transcription_id,
            cost_low: quantity * rate.cost_per_unit_low,
            cost_high: quantity * rate.cost_per_unit_high,
            cost_avg: quantity * rate.cost_per_unit_avg,
            created_at: chrono::Utc::now().fixed_offset(),
        },
    )
    .await?;

    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use entity::cost_metric::Metric;
    use entity::cost_pricing_config::Model as RateModel;
    use entity::cost_unit::Unit;
    use entity::meeting_recording::{MeetingRecordingStatus, Model as RecordingModel};
    use entity::pipeline_provider::Provider;
    use entity::platform_cost_metrics::Model as CostMetricsModel;
    use entity::Id;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::sync::Arc;

    fn test_rate(metric: Metric) -> RateModel {
        RateModel {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric,
            unit: Unit::Minutes,
            cost_per_unit_low: 0.001,
            cost_per_unit_high: 0.005,
            cost_per_unit_avg: 0.003,
            effective_from: chrono::Utc::now().fixed_offset(),
        }
    }

    fn test_recording(session_id: Id) -> RecordingModel {
        let now = chrono::Utc::now();
        RecordingModel {
            id: Id::new_v4(),
            coaching_session_id: session_id,
            bot_id: "bot-123".to_string(),
            status: MeetingRecordingStatus::Completed,
            video_url: None,
            audio_url: None,
            duration_seconds: Some(300),
            started_at: None,
            ended_at: None,
            error_message: None,
            created_at: now.into(),
            updated_at: now.into(),
        }
    }

    fn test_cost_metric(session_id: Id) -> CostMetricsModel {
        CostMetricsModel {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric: Metric::BotMinutes,
            coaching_session_id: Some(session_id),
            source_record_id: Id::new_v4(),
            cost_low: 0.005,
            cost_high: 0.025,
            cost_avg: 0.015,
            created_at: chrono::Utc::now().fixed_offset(),
        }
    }

    #[tokio::test]
    async fn record_bot_minutes_creates_cost_row_when_rate_configured() {
        let session_id = Id::new_v4();
        let recording = test_recording(session_id);
        let recording_id = recording.id;
        let rate = test_rate(Metric::BotMinutes);
        let cost_metric = test_cost_metric(session_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results(vec![vec![recording]])
                .append_query_results(vec![vec![cost_metric]])
                .into_connection(),
        );

        let result = super::record_bot_minutes(&db, recording_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_bot_minutes_noop_when_no_rate_configured() {
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results::<RateModel, Vec<RateModel>, _>(vec![vec![]])
                .into_connection(),
        );

        let result = super::record_bot_minutes(&db, Id::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_bot_minutes_noop_when_recording_not_found() {
        let rate = test_rate(Metric::BotMinutes);
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results::<RecordingModel, Vec<RecordingModel>, _>(vec![vec![]])
                .into_connection(),
        );

        let result = super::record_bot_minutes(&db, Id::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_bot_minutes_noop_when_duration_missing() {
        let session_id = Id::new_v4();
        let mut recording = test_recording(session_id);
        recording.duration_seconds = None;
        let recording_id = recording.id;
        let rate = test_rate(Metric::BotMinutes);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results(vec![vec![recording]])
                .into_connection(),
        );

        let result = super::record_bot_minutes(&db, recording_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_transcription_hours_creates_cost_row_when_rate_configured() {
        let session_id = Id::new_v4();
        let recording = test_recording(session_id);
        let recording_id = recording.id;
        let transcription_id = Id::new_v4();
        let rate = test_rate(Metric::TranscriptionHours);
        let cost_metric = test_cost_metric(session_id);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results(vec![vec![recording]])
                .append_query_results(vec![vec![cost_metric]])
                .into_connection(),
        );

        let result = super::record_transcription_hours(&db, transcription_id, recording_id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_transcription_hours_noop_when_no_rate_configured() {
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results::<RateModel, Vec<RateModel>, _>(vec![vec![]])
                .into_connection(),
        );

        let result = super::record_transcription_hours(&db, Id::new_v4(), Id::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_transcription_hours_noop_when_recording_not_found() {
        let rate = test_rate(Metric::TranscriptionHours);
        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results::<RecordingModel, Vec<RecordingModel>, _>(vec![vec![]])
                .into_connection(),
        );

        let result = super::record_transcription_hours(&db, Id::new_v4(), Id::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn record_transcription_hours_noop_when_duration_missing() {
        let session_id = Id::new_v4();
        let mut recording = test_recording(session_id);
        recording.duration_seconds = None;
        let recording_id = recording.id;
        let transcription_id = Id::new_v4();
        let rate = test_rate(Metric::TranscriptionHours);

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(vec![vec![rate]])
                .append_query_results(vec![vec![recording]])
                .into_connection(),
        );

        let result = super::record_transcription_hours(&db, transcription_id, recording_id).await;
        assert!(result.is_ok());
    }
}
