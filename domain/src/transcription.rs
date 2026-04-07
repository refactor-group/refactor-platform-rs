//! Business logic for meeting transcription lifecycle management.

pub use entity::transcription::{Model, TranscriptionStatus};
pub use entity_api::transcription::{find_by_coaching_session, find_by_external_id, update_status};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use crate::gateway::recall_ai;
use entity::meeting_recording::Model as RecordingModel;
use entity::transcript_segment::ActiveModel as SegmentActiveModel;
use entity::Id;
use entity_api::{
    meeting_recording as recording_api, transcript_segment as segment_api,
    transcription as transcription_api,
};
use log::*;
use sea_orm::{ActiveValue::Set, DatabaseConnection};
use service::config::Config;

fn recall_ai_provider(config: &Config) -> Result<recall_ai::Provider, Error> {
    let api_key = config.recall_ai_api_key().ok_or_else(|| {
        warn!("RECALL_AI_API_KEY not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;
    let webhook_base = config.webhook_base_url().ok_or_else(|| {
        warn!("WEBHOOK_BASE_URL not configured");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Config),
        }
    })?;
    let webhook_url = format!("{}/webhooks/recall_ai", webhook_base);
    recall_ai::Provider::new(&api_key, config.recall_ai_region(), &webhook_url)
}

/// Triggers async transcription for the given recording and persists the `transcriptions` row.
///
/// Called after `recording.done` webhook — passes the bot ID to Recall.ai's async transcript
/// endpoint, which uses AssemblyAI with speaker diarization. Completion is signaled via
/// `transcript.done` webhook.
pub async fn start(
    db: &DatabaseConnection,
    config: &Config,
    recording: &RecordingModel,
) -> Result<Model, Error> {
    let provider = recall_ai_provider(config)?;

    let transcript_id = provider.create_async_transcript(&recording.bot_id).await?;

    info!(
        "Created async transcript {} for session {}",
        transcript_id, recording.coaching_session_id
    );

    let now = chrono::Utc::now();
    let model = Model {
        id: Id::new_v4(),
        coaching_session_id: recording.coaching_session_id,
        meeting_recording_id: recording.id,
        external_id: transcript_id,
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

/// Fetches the completed transcript from Recall.ai and persists segments.
///
/// Called after `transcript.done` webhook:
/// 1. Retrieves transcript metadata (including `download_url`) from Recall.ai
/// 2. Downloads the transcript JSON
/// 3. Updates the `transcriptions` row with metadata
/// 4. Inserts all utterances as `transcript_segments`
pub async fn handle_completion(
    db: &DatabaseConnection,
    config: &Config,
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

    // Look up the recording to get the bot_id for the Recall.ai API call
    let recording =
        recording_api::find_by_bot_id(db, &transcription.meeting_recording_id.to_string()).await?;

    // The bot_id is the Recall.ai "recording ID" used in transcript API paths.
    let bot_id = recording
        .as_ref()
        .map(|r| r.bot_id.clone())
        .unwrap_or_else(|| transcription.meeting_recording_id.to_string());

    let provider = recall_ai_provider(config)?;

    let metadata = provider.get_async_transcript(&bot_id, external_id).await?;

    let download_url = metadata.download_url.ok_or_else(|| {
        warn!(
            "No download_url in transcript metadata for external_id={}",
            external_id
        );
        Error {
            source: None,
            error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                "Transcript download_url missing from Recall.ai response".to_string(),
            )),
        }
    })?;

    let transcript_data = provider.download_transcript(&download_url).await?;

    let word_count = metadata.word_count.map(|w| w as i32).or_else(|| {
        transcript_data.utterances.as_ref().map(|utterances| {
            utterances
                .iter()
                .map(|u| u.words.as_ref().map_or(0, |w| w.len()))
                .sum::<usize>() as i32
        })
    });

    transcription_api::update_status(
        db,
        transcription.id,
        TranscriptionStatus::Completed,
        word_count,
        metadata.confidence.or(transcript_data.confidence),
        None,
    )
    .await?;

    let utterances = transcript_data.utterances.unwrap_or_default();
    let segment_count = utterances.len();

    if utterances.is_empty() {
        warn!(
            "No utterances in transcript external_id={} — no segments inserted",
            external_id
        );
    } else {
        let now = chrono::Utc::now();
        let segments: Vec<SegmentActiveModel> = utterances
            .into_iter()
            .map(|u| SegmentActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription.id),
                speaker_label: Set(u.speaker),
                text: Set(u.text),
                start_ms: Set((u.start * 1000.0) as i32),
                end_ms: Set((u.end * 1000.0) as i32),
                confidence: Set(u.confidence),
                sentiment: Set(u.sentiment),
                created_at: Set(now.into()),
            })
            .collect();

        segment_api::create_batch(db, segments).await?;
    }

    info!(
        "Transcript completion handled for session_id={}: {} segments inserted",
        transcription.coaching_session_id, segment_count
    );

    Ok(())
}
