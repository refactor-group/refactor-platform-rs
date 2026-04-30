//! Webhook handler for Recall.ai events.
//!
//! Response code semantics for Svix retry logic:
//! - 200 OK: event received and processing started (or idempotent skip)
//! - 401 Unauthorized: signature invalid; Svix should not retry
//! - 500 Internal Server Error: DB failure during sync lookup; Svix will retry

use crate::AppState;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording::MeetingRecordingStatus;
use domain::transcription as TranscriptionApi;
use domain::transcription::TranscriptionStatus;
use domain::Id;
use log::*;
use meeting_auth::webhook::{SvixValidator, Validator};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

// ── Webhook event shape ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WebhookEvent {
    event: String,
    data: Value,
}

// ── Route handler ─────────────────────────────────────────────────────────────

/// POST /webhooks/recall_ai — receives all Recall.ai webhook events
pub async fn recall_ai(
    State(app_state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let secret = match app_state.config.recall_ai_webhook_secret() {
        Some(s) => s,
        None => {
            warn!("RECALL_AI_WEBHOOK_SECRET not configured — rejecting webhook");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    let validator = match SvixValidator::new("recall_ai".to_string(), &secret) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to build SvixValidator: {:?}", e);
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    let header_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|val| (k.as_str().to_lowercase(), val.to_string()))
        })
        .collect();

    match validator.validate(&header_map, &body) {
        Ok(true) => {}
        Ok(false) => {
            warn!("Invalid Svix signature: provider=recall_ai");
            return StatusCode::UNAUTHORIZED.into_response();
        }
        Err(e) => {
            warn!(
                "Svix validation error: provider=recall_ai svix-id={} error={:?}",
                header_map.get("svix-id").map(|s| s.as_str()).unwrap_or(""),
                e
            );
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    let event: WebhookEvent = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            warn!("Failed to deserialize Recall.ai webhook body: {:?}", e);
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    match event.event.as_str() {
        "bot.joining_call" => {
            handle_bot_status(app_state, event.data, MeetingRecordingStatus::Joining).await
        }
        "bot.in_waiting_room" => {
            handle_bot_status(app_state, event.data, MeetingRecordingStatus::WaitingRoom).await
        }
        "bot.in_call_not_recording" => {
            handle_bot_status(app_state, event.data, MeetingRecordingStatus::InMeeting).await
        }
        "bot.in_call_recording" => {
            handle_bot_status(app_state, event.data, MeetingRecordingStatus::Recording).await
        }
        "bot.done" => {
            handle_bot_status(app_state, event.data, MeetingRecordingStatus::Processing).await
        }
        "recording.done" => handle_recording_done(app_state, event.data).await,
        "transcript.done" => handle_transcript_done(app_state, event.data).await,
        "transcript.failed" => handle_transcript_failed(app_state, event.data).await,
        "bot.fatal" => handle_bot_fatal(app_state, event.data).await,
        other => {
            debug!("Unhandled Recall.ai webhook event: {}", other);
            StatusCode::OK.into_response()
        }
    }
}

// ── Event handlers ────────────────────────────────────────────────────────────

async fn handle_bot_status(
    app_state: AppState,
    data: Value,
    status: MeetingRecordingStatus,
) -> axum::response::Response {
    let bot_id = match data.pointer("/bot/id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            warn!("bot status event: missing /bot/id");
            return StatusCode::OK.into_response();
        }
    };

    let recording =
        match MeetingRecordingApi::find_by_bot_id(app_state.db_conn_ref(), &bot_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                warn!("bot status event: no recording for bot_id={}", bot_id);
                return StatusCode::OK.into_response();
            }
            Err(e) => {
                error!(
                    "bot status event: DB error for bot_id={}: {:?}",
                    bot_id, e
                );
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    if let Err(e) = MeetingRecordingApi::update_status(
        app_state.db_conn_ref(),
        recording.id,
        status,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
    {
        error!(
            "bot status event: failed to update recording {}: {:?}",
            recording.id, e
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::OK.into_response()
}

async fn handle_recording_done(app_state: AppState, data: Value) -> axum::response::Response {
    let bot_id = match data.pointer("/bot/id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            warn!("recording.done: missing /bot/id");
            return StatusCode::OK.into_response();
        }
    };

    let coaching_session_id_str = data
        .pointer("/bot/metadata/coaching_session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let coaching_session_id = match coaching_session_id_str.parse::<Id>() {
        Ok(id) => id,
        Err(_) => {
            warn!(
                "recording.done: invalid coaching_session_id '{}' in bot metadata",
                coaching_session_id_str
            );
            return StatusCode::OK.into_response();
        }
    };

    let recording =
        match MeetingRecordingApi::find_by_bot_id(app_state.db_conn_ref(), &bot_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                warn!("recording.done: no recording for bot_id={}", bot_id);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
            Err(e) => {
                error!("recording.done: DB error for bot_id={}: {:?}", bot_id, e);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    // Idempotency: skip if transcription already exists for this session
    match TranscriptionApi::find_by_coaching_session(app_state.db_conn_ref(), coaching_session_id)
        .await
    {
        Ok(Some(_)) => {
            warn!(
                "recording.done: transcription already exists for session={} — skipping",
                coaching_session_id
            );
            return StatusCode::OK.into_response();
        }
        Ok(None) => {}
        Err(e) => {
            error!(
                "recording.done: DB error checking transcription for session={}: {:?}",
                coaching_session_id, e
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    if let Err(e) = MeetingRecordingApi::update_status(
        app_state.db_conn_ref(),
        recording.id,
        MeetingRecordingStatus::Completed,
        None,
        None,
        None,
        None,
        Some(chrono::Utc::now().into()),
        None,
    )
    .await
    {
        error!(
            "recording.done: failed to update recording {}: {:?}",
            recording.id, e
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    let db = app_state.database_connection.clone();
    let config = app_state.config.clone();

    tokio::spawn(async move {
        if let Err(e) = domain::transcription::start(&db, &config, &recording).await {
            error!(
                "recording.done: transcription start failed for session={}: {:?}",
                coaching_session_id, e
            );
            let _ = MeetingRecordingApi::update_status(
                &db,
                recording.id,
                MeetingRecordingStatus::Failed,
                None,
                None,
                None,
                None,
                None,
                Some(e.to_string()),
            )
            .await;
        }
    });

    StatusCode::OK.into_response()
}

async fn handle_transcript_done(app_state: AppState, data: Value) -> axum::response::Response {
    let transcript_id = match data.pointer("/transcript/id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            warn!("transcript.done: missing /transcript/id");
            return StatusCode::OK.into_response();
        }
    };

    let transcription = match TranscriptionApi::find_by_external_id(
        app_state.db_conn_ref(),
        &transcript_id,
    )
    .await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                "transcript.done: no transcription for external_id={}",
                transcript_id
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => {
            error!(
                "transcript.done: DB error for external_id={}: {:?}",
                transcript_id, e
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Idempotency: already completed
    if transcription.status == TranscriptionStatus::Completed {
        warn!(
            "transcript.done: transcription {} already completed — skipping",
            transcription.id
        );
        return StatusCode::OK.into_response();
    }

    let transcription_id = transcription.id;
    let db = app_state.database_connection.clone();
    let config = app_state.config.clone();

    tokio::spawn(async move {
        if let Err(e) =
            domain::transcription::handle_completion(&db, &config, &transcript_id).await
        {
            error!(
                "transcript.done: completion failed for external_id={}: {:?}",
                transcript_id, e
            );
            let _ = TranscriptionApi::update_status(
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

    StatusCode::OK.into_response()
}

async fn handle_transcript_failed(app_state: AppState, data: Value) -> axum::response::Response {
    let transcript_id = match data.pointer("/transcript/id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            warn!("transcript.failed: missing /transcript/id");
            return StatusCode::OK.into_response();
        }
    };

    let error_message = data
        .pointer("/data/sub_code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let transcription = match TranscriptionApi::find_by_external_id(
        app_state.db_conn_ref(),
        &transcript_id,
    )
    .await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                "transcript.failed: no transcription for external_id={}",
                transcript_id
            );
            return StatusCode::OK.into_response();
        }
        Err(e) => {
            error!(
                "transcript.failed: DB error for external_id={}: {:?}",
                transcript_id, e
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = TranscriptionApi::update_status(
        app_state.db_conn_ref(),
        transcription.id,
        TranscriptionStatus::Failed,
        None,
        None,
        error_message,
    )
    .await
    {
        error!(
            "transcript.failed: failed to update transcription {}: {:?}",
            transcription.id, e
        );
    }

    StatusCode::OK.into_response()
}

async fn handle_bot_fatal(app_state: AppState, data: Value) -> axum::response::Response {
    let bot_id = match data.pointer("/bot/id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            warn!("bot.fatal: missing /bot/id");
            return StatusCode::OK.into_response();
        }
    };

    let error_message = data
        .pointer("/data/sub_code")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let recording =
        match MeetingRecordingApi::find_by_bot_id(app_state.db_conn_ref(), &bot_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                warn!("bot.fatal: no recording for bot_id={}", bot_id);
                return StatusCode::OK.into_response();
            }
            Err(e) => {
                error!("bot.fatal: DB error for bot_id={}: {:?}", bot_id, e);
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };

    if let Err(e) = MeetingRecordingApi::update_status(
        app_state.db_conn_ref(),
        recording.id,
        MeetingRecordingStatus::Failed,
        None,
        None,
        None,
        None,
        None,
        error_message,
    )
    .await
    {
        error!(
            "bot.fatal: failed to update recording {}: {:?}",
            recording.id, e
        );
    }

    StatusCode::OK.into_response()
}
