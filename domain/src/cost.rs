use crate::cost_metric::Metric;
use crate::error::Error;
use crate::pipeline_provider::Provider;
use crate::Id;
use entity_api::{cost_pricing_config, meeting_recording as recording_api, platform_cost_metrics};
use log::warn;
use sea_orm::DatabaseConnection;

/// Records the Recall.ai bot-minutes cost for a completed recording.
///
/// Fetches the recording fresh from the DB so `duration_seconds` reflects the
/// value written by `try_claim_completed`. No-ops with a warning if no pricing
/// row is configured.
pub async fn record_bot_minutes(db: &DatabaseConnection, recording_id: Id) -> Result<(), Error> {
    // The recording is both the cost source record and the duration source.
    record(db, Metric::BotMinutes, recording_id, recording_id).await
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
    // Cost is attributed to the transcription, but duration comes from the recording.
    record(
        db,
        Metric::TranscriptionHours,
        transcription_id,
        meeting_recording_id,
    )
    .await
}

/// Shared cost-recording path for the duration-derived Recall.ai metrics.
///
/// Looks up the current rate, fetches the recording that carries the billable
/// duration, derives the quantity in the rate's unit, and writes one cost row.
/// No-ops with a warning at each missing-data gate (no rate, no recording, no
/// duration, or a unit not derivable from a duration) rather than writing a
/// misleading `$0.00` row. The rate lookup runs first so a missing rate skips
/// the recording fetch entirely.
async fn record(
    db: &DatabaseConnection,
    metric: Metric,
    source_record_id: Id,
    meeting_recording_id: Id,
) -> Result<(), Error> {
    let Some(rate) = cost_pricing_config::find_current_rate(db, Provider::RecallAi, metric).await?
    else {
        warn!("cost: no pricing config for (RecallAi, {metric:?}) — skipping {source_record_id}");
        return Ok(());
    };

    let Some(recording) = recording_api::find_by_id(db, meeting_recording_id).await? else {
        warn!(
            "cost: recording {} not found — skipping {:?} cost for {}",
            meeting_recording_id, metric, source_record_id
        );
        return Ok(());
    };

    // `None` here means nothing to bill — a missing/non-positive duration or a
    // non-duration unit — so we skip rather than write a misleading $0.00 row.
    let Some(quantity) = rate.unit.quantity_from_seconds(recording.duration_seconds) else {
        warn!(
            "cost: no billable quantity for {:?} (recording {}, unit {:?}, duration {:?}) — skipping {}",
            metric, meeting_recording_id, rate.unit, recording.duration_seconds, source_record_id
        );
        return Ok(());
    };

    platform_cost_metrics::create(
        db,
        platform_cost_metrics::CreateParams {
            provider: Provider::RecallAi,
            metric,
            coaching_session_id: Some(recording.coaching_session_id),
            source_record_id,
            cost_low: quantity * rate.cost_per_unit_low,
            cost_high: quantity * rate.cost_per_unit_high,
            cost_avg: quantity * rate.cost_per_unit_avg,
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
    use sea_orm::prelude::Decimal;
    use sea_orm::{DatabaseBackend, MockDatabase};
    use std::sync::Arc;

    fn test_rate(metric: Metric) -> RateModel {
        let unit = match metric {
            Metric::BotMinutes => Unit::Minutes,
            Metric::TranscriptionHours => Unit::Hours,
            Metric::LlmTokens => Unit::Tokens,
        };
        RateModel {
            id: Id::new_v4(),
            provider: Provider::RecallAi,
            metric,
            unit,
            cost_per_unit_low: Decimal::new(1, 3),
            cost_per_unit_high: Decimal::new(5, 3),
            cost_per_unit_avg: Decimal::new(3, 3),
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
            cost_low: Decimal::new(5, 3),
            cost_high: Decimal::new(25, 3),
            cost_avg: Decimal::new(15, 3),
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
