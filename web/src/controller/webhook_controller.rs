//! Controller for handling webhooks from external services.
//!
//! Handles webhooks from Recall.ai for meeting recording status updates.

use crate::{AppState, Error};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use domain::gateway::assembly_ai::TranscriptStatus;
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording_status::MeetingRecordingStatus;
use domain::meeting_recordings::Model as MeetingRecordingModel;
use domain::transcript_segment::{self, SegmentInput};
use domain::transcription as TranscriptionApi;
use domain::transcription_status::TranscriptionStatus;
use domain::transcriptions::Model as TranscriptionModel;
use log::*;
use serde::{Deserialize, Serialize};

/// Recall.ai webhook event payload
#[derive(Debug, Deserialize)]
pub struct RecallWebhookPayload {
    /// The type of event
    pub event: String,
    /// The bot ID this event is for
    pub data: RecallWebhookData,
}

/// Data section of Recall.ai webhook
#[derive(Debug, Deserialize)]
pub struct RecallWebhookData {
    /// Bot ID
    pub bot_id: String,
    /// Status code (for status change events)
    pub status: Option<RecallBotStatus>,
    /// Video URL (available when recording is complete)
    pub video_url: Option<String>,
    /// Recording duration in seconds
    pub duration: Option<i32>,
    /// Error details if the bot failed
    pub error: Option<RecallError>,
}

/// Recall.ai bot status
#[derive(Debug, Deserialize)]
pub struct RecallBotStatus {
    pub code: String,
}

/// Recall.ai error details
#[derive(Debug, Deserialize)]
pub struct RecallError {
    pub code: Option<String>,
    pub message: Option<String>,
}

/// Response for webhook acknowledgment
#[derive(Debug, Serialize)]
pub struct WebhookResponse {
    pub status: String,
}

/// Maps Recall.ai status codes to our internal status
fn map_recall_status(code: &str) -> MeetingRecordingStatus {
    match code {
        "ready" | "joining_call" => MeetingRecordingStatus::Joining,
        "in_call_not_recording" | "in_waiting_room" => MeetingRecordingStatus::Joining,
        "in_call_recording" => MeetingRecordingStatus::Recording,
        "call_ended" | "done" => MeetingRecordingStatus::Processing,
        "analysis_done" => MeetingRecordingStatus::Completed,
        "fatal" | "error" => MeetingRecordingStatus::Failed,
        _ => MeetingRecordingStatus::Pending,
    }
}

/// POST /webhooks/recall
///
/// Handles webhook callbacks from Recall.ai for bot status updates.
/// This endpoint does not require authentication but validates via webhook secret.
pub async fn recall_webhook(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RecallWebhookPayload>,
) -> Result<impl IntoResponse, Error> {
    debug!("Received Recall.ai webhook: {:?}", payload.event);

    let config = &app_state.config;
    let db = app_state.db_conn_ref();

    // Validate webhook secret if configured
    if let Some(expected_secret) = config.webhook_secret() {
        let provided_secret = headers
            .get("x-webhook-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided_secret != expected_secret {
            warn!("Invalid webhook secret received");
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(WebhookResponse {
                    status: "unauthorized".to_string(),
                }),
            ));
        }
    }

    let bot_id = &payload.data.bot_id;

    // Find the recording by bot ID
    let recording: Option<MeetingRecordingModel> =
        MeetingRecordingApi::find_by_recall_bot_id(db, bot_id).await?;

    let recording = match recording {
        Some(r) => r,
        None => {
            warn!("Received webhook for unknown bot ID: {}", bot_id);
            return Ok((
                StatusCode::OK,
                Json(WebhookResponse {
                    status: "ignored".to_string(),
                }),
            ));
        }
    };

    // Handle different event types
    match payload.event.as_str() {
        "bot.status_change" => {
            if let Some(status) = &payload.data.status {
                let new_status = map_recall_status(&status.code);
                info!(
                    "Bot {} status changed to {} (internal: {:?})",
                    bot_id, status.code, new_status
                );

                // Check for errors
                let error_message = if new_status == MeetingRecordingStatus::Failed {
                    payload.data.error.as_ref().map(|e| {
                        format!(
                            "{}: {}",
                            e.code.as_deref().unwrap_or("unknown"),
                            e.message.as_deref().unwrap_or("Unknown error")
                        )
                    })
                } else {
                    None
                };

                let _: MeetingRecordingModel =
                    MeetingRecordingApi::update_status(db, recording.id, new_status, error_message)
                        .await?;
            }
        }
        "bot.done" | "recording.done" => {
            info!("Bot {} recording completed", bot_id);

            // Update with video URL and duration if available
            let mut updated_recording = recording.clone();
            updated_recording.status = MeetingRecordingStatus::Completed;
            updated_recording.recording_url = payload.data.video_url.clone();
            updated_recording.duration_seconds = payload.data.duration;
            updated_recording.ended_at = Some(chrono::Utc::now().into());

            let _: MeetingRecordingModel =
                MeetingRecordingApi::update(db, recording.id, updated_recording).await?;
        }
        "bot.error" | "recording.error" => {
            warn!("Bot {} encountered an error", bot_id);

            let error_message = payload.data.error.as_ref().map(|e| {
                format!(
                    "{}: {}",
                    e.code.as_deref().unwrap_or("unknown"),
                    e.message.as_deref().unwrap_or("Unknown error")
                )
            });

            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                recording.id,
                MeetingRecordingStatus::Failed,
                error_message,
            )
            .await?;
        }
        _ => {
            debug!("Ignoring unhandled Recall.ai event: {}", payload.event);
        }
    }

    Ok((
        StatusCode::OK,
        Json(WebhookResponse {
            status: "ok".to_string(),
        }),
    ))
}

/// AssemblyAI webhook payload
/// AssemblyAI sends the full transcript response when transcription is complete
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields are part of AssemblyAI API contract
pub struct AssemblyAiWebhookPayload {
    /// The transcript ID
    pub transcript_id: String,
    /// Status: queued, processing, completed, error
    pub status: TranscriptStatus,
    /// Full text of the transcript (available when completed)
    #[serde(default)]
    pub text: Option<String>,
    /// Speaker-labeled utterances
    #[serde(default)]
    pub utterances: Option<Vec<AssemblyAiUtterance>>,
    /// Auto-generated chapters/summary
    #[serde(default)]
    pub chapters: Option<Vec<AssemblyAiChapter>>,
    /// Confidence score
    #[serde(default)]
    pub confidence: Option<f64>,
    /// Audio duration in milliseconds
    #[serde(default)]
    pub audio_duration: Option<i64>,
    /// Error message if failed
    #[serde(default)]
    pub error: Option<String>,
}

/// AssemblyAI utterance (speaker segment)
#[derive(Debug, Deserialize)]
pub struct AssemblyAiUtterance {
    pub text: String,
    pub start: i64,
    pub end: i64,
    pub confidence: f64,
    pub speaker: String,
}

/// AssemblyAI chapter for summary
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // Fields are part of AssemblyAI API contract
pub struct AssemblyAiChapter {
    pub summary: String,
    pub headline: String,
    pub start: i64,
    pub end: i64,
    pub gist: String,
}

/// POST /webhooks/assemblyai
///
/// Handles webhook callbacks from AssemblyAI when transcription is complete.
/// This endpoint validates via webhook secret header.
pub async fn assemblyai_webhook(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AssemblyAiWebhookPayload>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "Received AssemblyAI webhook for transcript: {}",
        payload.transcript_id
    );

    let config = &app_state.config;
    let db = app_state.db_conn_ref();

    // Validate webhook secret if configured
    if let Some(expected_secret) = config.webhook_secret() {
        let provided_secret = headers
            .get("x-webhook-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if provided_secret != expected_secret {
            warn!("Invalid AssemblyAI webhook secret received");
            return Ok((
                StatusCode::UNAUTHORIZED,
                Json(WebhookResponse {
                    status: "unauthorized".to_string(),
                }),
            ));
        }
    }

    // Find the transcription by AssemblyAI transcript ID
    let transcription: Option<TranscriptionModel> =
        TranscriptionApi::find_by_assemblyai_id(db, &payload.transcript_id).await?;

    let transcription = match transcription {
        Some(t) => t,
        None => {
            warn!(
                "Received webhook for unknown AssemblyAI transcript: {}",
                payload.transcript_id
            );
            return Ok((
                StatusCode::OK,
                Json(WebhookResponse {
                    status: "ignored".to_string(),
                }),
            ));
        }
    };

    match payload.status {
        TranscriptStatus::Completed => {
            info!(
                "AssemblyAI transcription completed: {}",
                payload.transcript_id
            );

            // Build summary from chapters if available
            let summary = payload.chapters.as_ref().map(|chapters| {
                chapters
                    .iter()
                    .map(|c| format!("**{}**\n{}", c.headline, c.summary))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            });

            // Calculate word count from full text
            let word_count = payload
                .text
                .as_ref()
                .map(|t| t.split_whitespace().count() as i32);

            // Update transcription with completed data
            let mut updated = transcription.clone();
            updated.status = TranscriptionStatus::Completed;
            updated.full_text = payload.text.clone();
            updated.summary = summary;
            updated.confidence_score = payload.confidence;
            updated.word_count = word_count;

            let updated_transcription: TranscriptionModel =
                TranscriptionApi::update(db, transcription.id, updated).await?;

            // Store transcript segments (utterances) if available
            if let Some(utterances) = payload.utterances {
                let utterance_count = utterances.len();
                let segments: Vec<SegmentInput> = utterances
                    .into_iter()
                    .map(|u| SegmentInput {
                        speaker_label: u.speaker.clone(),
                        text: u.text,
                        start_time_ms: u.start,
                        end_time_ms: u.end,
                        confidence: Some(u.confidence),
                        sentiment: None, // Would need sentiment_analysis_results for per-segment sentiment
                    })
                    .collect();

                if !segments.is_empty() {
                    let _ =
                        transcript_segment::create_batch(db, updated_transcription.id, segments)
                            .await?;
                    info!(
                        "Created {} transcript segments for transcription {}",
                        utterance_count, updated_transcription.id
                    );
                }
            }

            // Update meeting recording status to completed
            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                transcription.meeting_recording_id,
                MeetingRecordingStatus::Completed,
                None,
            )
            .await?;
        }
        TranscriptStatus::Error => {
            warn!("AssemblyAI transcription failed: {}", payload.transcript_id);

            let _: TranscriptionModel = TranscriptionApi::update_status(
                db,
                transcription.id,
                TranscriptionStatus::Failed,
                payload.error.clone(),
            )
            .await?;

            // Update meeting recording with error
            let _: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                transcription.meeting_recording_id,
                MeetingRecordingStatus::Failed,
                payload.error,
            )
            .await?;
        }
        TranscriptStatus::Processing | TranscriptStatus::Queued => {
            debug!(
                "AssemblyAI transcription still processing: {}",
                payload.transcript_id
            );
            // No action needed - these are status updates during processing
        }
    }

    Ok((
        StatusCode::OK,
        Json(WebhookResponse {
            status: "ok".to_string(),
        }),
    ))
}
