//! Business logic for meeting transcription lifecycle management.

pub use entity::transcription::{Model, TranscriptionStatus};
pub use entity_api::transcription::{find_by_coaching_session, find_by_external_id, update_status};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use crate::gateway::recall_ai;
use entity::meeting_recording::Model as RecordingModel;
use entity::transcript_segment::ActiveModel as SegmentActiveModel;
use entity::Id;
use entity_api::{transcript_segment as segment_api, transcription as transcription_api};
use log::*;
use sea_orm::{ActiveValue::Set, DatabaseConnection};
use service::config::Config;

/// Maximum silence gap between consecutive words from the same speaker before
/// starting a new segment. Matches Recall.ai's own recommended coalescing approach.
const SEGMENT_GAP_SECS: f64 = 1.5;

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
/// Called after `recording.done` webhook. `recall_recording_id` is Recall's recording UUID
/// (`data.recording.id` from the webhook) used for all subsequent transcript API calls.
pub async fn start(
    db: &DatabaseConnection,
    config: &Config,
    recording: &RecordingModel,
    recall_recording_id: &str,
) -> Result<Model, Error> {
    let provider = recall_ai_provider(config)?;

    let transcript_id = provider
        .create_async_transcript(recall_recording_id)
        .await?;

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

    let recall_recording_id = transcription.recall_recording_id.as_deref().ok_or_else(|| {
        warn!(
            "transcript {} has no recall_recording_id — cannot fetch from Recall.ai",
            transcription.id
        );
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::NotFound,
            )),
        }
    })?;

    let provider = recall_ai_provider(config)?;

    let metadata = provider
        .get_async_transcript(recall_recording_id, external_id)
        .await?;

    let download_url = metadata.download_url().ok_or_else(|| {
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

    let participant_entries = provider.download_transcript(download_url).await?;

    // Collect every word across all participants, tagging each with its speaker label.
    // Prefer participant name; fall back to numeric ID; fall back to "Unknown".
    struct WordEntry {
        speaker: String,
        text: String,
        start_s: f64,
        end_s: f64,
    }

    let mut all_words: Vec<WordEntry> = Vec::new();
    for entry in &participant_entries {
        let speaker = entry
            .participant
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| n.to_string())
            .or_else(|| entry.participant.id.map(|id| id.to_string()))
            .unwrap_or_else(|| "Unknown".to_string());

        for word in &entry.words {
            let start_s = word.start_timestamp.relative.unwrap_or(0.0);
            let end_s = word.end_timestamp.relative.unwrap_or(start_s);
            all_words.push(WordEntry {
                speaker: speaker.clone(),
                text: word.text.clone(),
                start_s,
                end_s,
            });
        }
    }

    let word_count = all_words.len();

    // Sort chronologically — participant entries are grouped by speaker, not by time.
    all_words.sort_by(|a, b| {
        a.start_s
            .partial_cmp(&b.start_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Coalesce adjacent same-speaker words into segments, splitting on speaker change
    // or a silence gap. This mirrors Recall.ai's own recommended approach from their FAQ.
    let mut segments: Vec<(String, String, f64, f64)> = Vec::new();
    if let Some(first) = all_words.first() {
        let mut seg_speaker = first.speaker.clone();
        let mut seg_words: Vec<String> = vec![first.text.clone()];
        let mut seg_start = first.start_s;
        let mut seg_end = first.end_s;

        for word in all_words.iter().skip(1) {
            let same_speaker = word.speaker == seg_speaker;
            let small_gap = word.start_s - seg_end < SEGMENT_GAP_SECS;

            if same_speaker && small_gap {
                seg_words.push(word.text.clone());
                seg_end = word.end_s;
            } else {
                segments.push((
                    seg_speaker.clone(),
                    seg_words.join(" "),
                    seg_start,
                    seg_end,
                ));
                seg_speaker = word.speaker.clone();
                seg_words = vec![word.text.clone()];
                seg_start = word.start_s;
                seg_end = word.end_s;
            }
        }
        segments.push((seg_speaker, seg_words.join(" "), seg_start, seg_end));
    }

    let segment_count = segments.len();

    transcription_api::update_status(
        db,
        transcription.id,
        TranscriptionStatus::Completed,
        Some(word_count as i32),
        None,
        None,
    )
    .await?;

    if segments.is_empty() {
        warn!(
            "No words in transcript external_id={} — no segments inserted",
            external_id
        );
    } else {
        let now = chrono::Utc::now();
        let segment_models: Vec<SegmentActiveModel> = segments
            .into_iter()
            .map(|(speaker, text, start_s, end_s)| SegmentActiveModel {
                id: Set(Id::new_v4()),
                transcription_id: Set(transcription.id),
                speaker_label: Set(speaker),
                text: Set(text),
                start_ms: Set((start_s * 1000.0) as i32),
                end_ms: Set((end_s * 1000.0) as i32),
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
