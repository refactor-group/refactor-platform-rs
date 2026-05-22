//! Business logic for meeting transcription lifecycle management.

pub use entity::transcription::{Model, TranscriptionStatus};
pub use entity_api::transcription::{
    find_by_coaching_session, find_by_external_id, find_by_id, try_claim_for_processing,
    update_status,
};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use entity::meeting_recording::Model as RecordingModel;
use entity::transcript_segment::ActiveModel as SegmentActiveModel;
use entity::Id;
use entity_api::{transcript_segment as segment_api, transcription as transcription_api};
use log::*;
use meeting_ai::traits::transcription as transcription_trait;
use meeting_ai::types::transcription as transcription_types;
use sea_orm::{ActiveValue::Set, DatabaseConnection};
use std::collections::HashMap;

/// Triggers async transcription for the given recording and persists the `transcriptions` row.
///
/// Called after `recording.done` webhook. `recall_recording_id` is the recording UUID
/// used for all subsequent transcript API calls.
pub async fn start(
    db: &DatabaseConnection,
    provider: Option<&dyn transcription_trait::Provider>,
    recording: &RecordingModel,
    recall_recording_id: &str,
) -> Result<Model, Error> {
    let provider = provider.ok_or_else(|| {
        warn!("Transcription provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let mut provider_options = HashMap::new();
    provider_options.insert(
        "recall_recording_id".to_string(),
        recall_recording_id.to_string(),
    );

    let config = transcription_types::Config {
        media_url: String::new(),
        webhook_url: None,
        enable_speaker_labels: true,
        enable_sentiment_analysis: false,
        enable_auto_chapters: false,
        enable_entity_detection: false,
        language_code: None,
        provider_options,
    };

    let transcription = provider
        .create_transcription(config)
        .await
        .map_err(Error::from)?;

    info!(
        "Created async transcript {} for session {}",
        transcription.id, recording.coaching_session_id
    );

    let now = chrono::Utc::now();
    let model = Model {
        id: Id::new_v4(),
        coaching_session_id: recording.coaching_session_id,
        meeting_recording_id: recording.id,
        external_id: transcription.id,
        recall_recording_id: Some(recall_recording_id.to_string()),
        status: TranscriptionStatus::Queued,
        language_code: None,
        speaker_count: None,
        word_count: None,
        duration_seconds: None,
        confidence: None,
        error_message: None,
        created_at: now.into(),
        updated_at: now.into(),
    };

    Ok(transcription_api::create(db, model).await?)
}

/// Fetches the completed transcript from the provider and persists segments.
///
/// Called after `transcript.done` webhook:
/// 1. Retrieves coalesced transcript segments from the provider
/// 2. Updates the `transcriptions` row with word count and Completed status
/// 3. Inserts all utterance segments as `transcript_segments`
pub async fn handle_completion(
    db: &DatabaseConnection,
    provider: Option<&dyn transcription_trait::Provider>,
    external_id: &str,
) -> Result<(), Error> {
    info!(
        "Handling transcript completion for external_id={}",
        external_id
    );

    let transcription = transcription_api::find_by_external_id(db, external_id)
        .await?
        .ok_or_else(|| Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::NotFound,
            )),
        })?;

    let provider = provider.ok_or_else(|| {
        warn!("Transcription provider not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;

    let result = provider
        .get_transcription(external_id)
        .await
        .map_err(Error::from)?;

    let word_count: usize = result
        .segments
        .iter()
        .map(|s| s.text.split_whitespace().count())
        .sum();

    let segment_count = result.segments.len();

    transcription_api::update_status(
        db,
        transcription.id,
        TranscriptionStatus::Completed,
        Some(i32::try_from(word_count).unwrap_or(i32::MAX)),
        None,
        None,
    )
    .await?;

    if result.segments.is_empty() {
        warn!(
            "No segments in transcript external_id={} — no segments inserted",
            external_id
        );
    } else {
        let now = chrono::Utc::now();
        let segment_models: Vec<SegmentActiveModel> = result
            .segments
            .into_iter()
            .map(|seg| SegmentActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription.id),
                speaker_label: Set(seg.speaker),
                text: Set(seg.text),
                start_ms: Set(i32::try_from(seg.start_ms).unwrap_or(i32::MAX)),
                end_ms: Set(i32::try_from(seg.end_ms).unwrap_or(i32::MAX)),
                confidence: Set(None),
                sentiment: Set(None),
                created_at: Set(now.into()),
            })
            .collect();

        segment_api::create_batch(db, segment_models).await?;
    }

    info!(
        "Transcript completion handled for session_id={}: {} segments inserted",
        transcription.coaching_session_id, segment_count
    );

    Ok(())
}
