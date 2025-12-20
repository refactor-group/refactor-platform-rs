//! Controller for handling webhooks from external services.
//!
//! Handles webhooks from Recall.ai for meeting recording status updates.

use crate::{AppState, Error};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;

use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording_status::MeetingRecordingStatus;
use domain::meeting_recordings::Model as MeetingRecordingModel;
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
