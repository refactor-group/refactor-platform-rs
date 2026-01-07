//! Controller for meeting recording operations.
//!
//! Handles starting, stopping, and querying meeting recordings via Recall.ai.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::ai_privacy_level::AiPrivacyLevel;
use domain::coaching_relationship as CoachingRelationshipApi;
use domain::coaching_session as CoachingSessionApi;
use domain::gateway::recall_ai::{create_standard_bot_request, RecallAiClient, RecallRegion};
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording_status::MeetingRecordingStatus;
use domain::meeting_recordings::Model as MeetingRecordingModel;
use domain::{user_integration, Id};
use log::*;
use service::config::ApiVersion;

/// Helper to create a forbidden error
fn forbidden_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}

/// Helper to create a bad request error
fn bad_request_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}

/// Helper to create an internal error
fn internal_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Other(message.to_string()),
        ),
    })
}

/// GET /coaching_sessions/{id}/recording
///
/// Get the current recording status for a coaching session.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/recording",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Recording status retrieved", body = meeting_recordings::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No recording found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_recording_status(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET recording status for session: {session_id}");

    let db = app_state.db_conn_ref();
    let config = &app_state.config;

    let recording: Option<MeetingRecordingModel> =
        MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id).await?;

    match recording {
        Some(rec) => {
            // If recording is in a transitional state and has a bot ID, poll Recall.ai for updates
            let should_poll = matches!(
                rec.status,
                MeetingRecordingStatus::Joining
                    | MeetingRecordingStatus::Recording
                    | MeetingRecordingStatus::Processing
            ) && rec.recall_bot_id.is_some();

            if should_poll {
                // Try to poll Recall.ai for the latest status
                if let Some(updated_rec) = poll_recall_for_status(db, config, &rec, user.id).await {
                    return Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_rec)));
                }
            }

            Ok(Json(ApiResponse::new(StatusCode::OK.into(), rec)))
        }
        None => Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::NotFound),
            ),
        })),
    }
}

/// POST /coaching_sessions/{id}/recording/start
///
/// Start recording a coaching session via Recall.ai bot.
/// Only the coach can start recording, and AI features must be enabled for the relationship.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{id}/recording/start",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 201, description = "Recording started successfully", body = meeting_recordings::Model),
        (status = 400, description = "Cannot start recording (AI disabled or no meeting URL)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the coach can start recording"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn start_recording(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("Starting recording for session: {session_id}");

    let db = app_state.db_conn_ref();
    let config = &app_state.config;

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // 3. Verify user is the coach
    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to start recording for session {} but is not the coach",
            user.id, session_id
        );
        return Err(forbidden_error("Only the coach can start recording"));
    }

    // 4. Check effective AI privacy level (minimum of coach and coachee consent)
    let effective_level = relationship
        .coach_ai_privacy_level
        .min_level(relationship.coachee_ai_privacy_level);
    if effective_level == AiPrivacyLevel::None {
        return Err(bad_request_error(
            "AI recording requires consent from both coach and coachee. Please check privacy settings.",
        ));
    }

    // 5. Check for meeting URL
    let meeting_url = relationship.meeting_url.clone().ok_or_else(|| {
        bad_request_error("No meeting URL configured for this coaching relationship")
    })?;

    // 6. Get user's Recall.ai API key
    let user_integrations = user_integration::find_by_user_id(db, user.id)
        .await?
        .ok_or_else(|| {
            bad_request_error("No integrations configured. Please set up Recall.ai in Settings.")
        })?;

    let api_key = user_integrations.recall_ai_api_key.clone().ok_or_else(|| {
        bad_request_error("Recall.ai API key not configured. Please set up in Settings.")
    })?;

    // 7. Determine region
    let region_str = user_integrations
        .recall_ai_region
        .as_deref()
        .unwrap_or("us-west-2");
    let region: RecallRegion = region_str.parse().unwrap_or(RecallRegion::UsWest2);

    // 8. Create meeting recording record
    let mut recording: MeetingRecordingModel = MeetingRecordingApi::create(db, session_id).await?;

    // 9. Create Recall.ai bot
    let client = RecallAiClient::new(&api_key, region, config.recall_ai_base_domain())?;

    // Build webhook URL for status updates
    let webhook_url = config
        .webhook_base_url()
        .map(|base| format!("{}/webhooks/recall", base));

    let bot_request = create_standard_bot_request(
        meeting_url,
        "Refactor Coaching Notetaker".to_string(),
        webhook_url,
    );

    match client.create_bot(bot_request).await {
        Ok(response) => {
            info!(
                "Recall.ai bot created: {} for session {}",
                response.id, session_id
            );

            // Update recording with bot ID and status
            recording.recall_bot_id = Some(response.id);
            recording.status = MeetingRecordingStatus::Joining;
            recording.started_at = Some(chrono::Utc::now().into());

            let updated: MeetingRecordingModel =
                MeetingRecordingApi::update(db, recording.id, recording).await?;

            Ok((
                StatusCode::CREATED,
                Json(ApiResponse::new(StatusCode::CREATED.into(), updated)),
            ))
        }
        Err(e) => {
            warn!(
                "Failed to create Recall.ai bot for session {}: {:?}",
                session_id, e
            );

            // Update recording with error status
            recording.status = MeetingRecordingStatus::Failed;
            recording.error_message = Some(format!("Failed to create bot: {:?}", e));
            let _ = MeetingRecordingApi::update(db, recording.id, recording).await;

            Err(internal_error("Failed to start recording"))
        }
    }
}

/// POST /coaching_sessions/{id}/recording/stop
///
/// Stop an active recording for a coaching session.
/// Only the coach can stop recording.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{id}/recording/stop",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Recording stopped successfully", body = meeting_recordings::Model),
        (status = 400, description = "No active recording to stop"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the coach can stop recording"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn stop_recording(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("Stopping recording for session: {session_id}");

    let db = app_state.db_conn_ref();
    let config = &app_state.config;

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // 3. Verify user is the coach
    if relationship.coach_id != user.id {
        warn!(
            "User {} attempted to stop recording for session {} but is not the coach",
            user.id, session_id
        );
        return Err(forbidden_error("Only the coach can stop recording"));
    }

    // 4. Get the active recording
    let recording: MeetingRecordingModel =
        MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
            .await?
            .ok_or_else(|| bad_request_error("No recording found for this session"))?;

    // 5. Check if recording is active
    if !matches!(
        recording.status,
        MeetingRecordingStatus::Joining | MeetingRecordingStatus::Recording
    ) {
        return Err(bad_request_error("No active recording to stop"));
    }

    // 6. Get bot ID
    let bot_id = recording
        .recall_bot_id
        .clone()
        .ok_or_else(|| internal_error("Recording has no associated bot ID"))?;

    // 7. Get user's Recall.ai API key
    let user_integrations = user_integration::find_by_user_id(db, user.id)
        .await?
        .ok_or_else(|| internal_error("User integrations not found"))?;

    let api_key = user_integrations
        .recall_ai_api_key
        .clone()
        .ok_or_else(|| internal_error("Recall.ai API key not found"))?;

    // 8. Determine region
    let region_str = user_integrations
        .recall_ai_region
        .as_deref()
        .unwrap_or("us-west-2");
    let region: RecallRegion = region_str.parse().unwrap_or(RecallRegion::UsWest2);

    // 9. Stop the Recall.ai bot
    let client = RecallAiClient::new(&api_key, region, config.recall_ai_base_domain())?;

    match client.stop_bot(&bot_id).await {
        Ok(_) => {
            info!(
                "Recall.ai bot stopped: {} for session {}",
                bot_id, session_id
            );

            // Update recording status
            let updated: MeetingRecordingModel = MeetingRecordingApi::update_status(
                db,
                recording.id,
                MeetingRecordingStatus::Processing,
                None,
            )
            .await?;

            Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
        }
        Err(e) => {
            warn!(
                "Failed to stop Recall.ai bot {} for session {}: {:?}",
                bot_id, session_id, e
            );
            Err(internal_error("Failed to stop recording"))
        }
    }
}

/// Poll Recall.ai for the latest bot status and update our database.
///
/// Returns the updated recording if status changed, None otherwise.
async fn poll_recall_for_status(
    db: &sea_orm::DatabaseConnection,
    config: &service::config::Config,
    recording: &MeetingRecordingModel,
    user_id: Id,
) -> Option<MeetingRecordingModel> {
    let bot_id = recording.recall_bot_id.as_ref()?;

    debug!("Polling Recall.ai for bot: {}", bot_id);

    // Get user's Recall.ai API key
    let user_integrations = match user_integration::find_by_user_id(db, user_id).await {
        Ok(Some(ui)) => ui,
        Ok(None) => {
            debug!("No user integrations found for user {}", user_id);
            return None;
        }
        Err(e) => {
            warn!("Error fetching user integrations: {:?}", e);
            return None;
        }
    };

    let api_key = match user_integrations.recall_ai_api_key.as_ref() {
        Some(key) => key,
        None => {
            debug!("No Recall.ai API key configured for user {}", user_id);
            return None;
        }
    };

    let region_str = user_integrations
        .recall_ai_region
        .as_deref()
        .unwrap_or("us-west-2");
    let region: RecallRegion = region_str.parse().unwrap_or(RecallRegion::UsWest2);

    // Create client and fetch bot status
    let client = match RecallAiClient::new(api_key, region, config.recall_ai_base_domain()) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to create Recall.ai client: {:?}", e);
            return None;
        }
    };

    let status_response = match client.get_bot_status(bot_id).await {
        Ok(r) => {
            debug!("Recall.ai response: {:?}", r);
            r
        }
        Err(e) => {
            warn!("Failed to get bot status from Recall.ai: {:?}", e);
            return None;
        }
    };

    // Map Recall.ai status to our internal status
    let latest_status_code = status_response
        .status_changes
        .last()
        .map(|s| s.code.as_str())
        .unwrap_or("unknown");

    let new_status = map_recall_status_code(latest_status_code);

    // Extract video URL using the helper method (handles nested structure)
    let video_url = status_response.video_url();

    debug!(
        "Recall.ai bot {} - recordings count: {}, video_url extracted: {:?}",
        bot_id,
        status_response.recordings.len(),
        video_url.as_ref().map(|u| &u[..50.min(u.len())])
    );

    // Check if status changed or video URL is now available
    if new_status == recording.status && video_url.is_none() {
        debug!("No status change and no video URL - skipping update");
        return None; // No change
    }

    info!(
        "Recall.ai bot {} status: {} -> {:?}, video_url: {}",
        bot_id,
        latest_status_code,
        new_status,
        video_url.is_some()
    );

    // Update recording with new status
    let mut updated = recording.clone();
    updated.status = new_status;

    // If recording is complete, capture video URL and duration
    if let Some(url) = video_url {
        updated.recording_url = Some(url);
        updated.status = MeetingRecordingStatus::Processing;
        updated.ended_at = Some(chrono::Utc::now().into());

        // Get duration from the recording object
        if let Some(duration) = status_response.duration_seconds() {
            updated.duration_seconds = Some(duration);
        }
    }

    // Save the updated recording
    let saved = MeetingRecordingApi::update(db, recording.id, updated)
        .await
        .ok()?;

    // If recording is complete with video URL, trigger AssemblyAI transcription
    if saved.status == MeetingRecordingStatus::Processing {
        if let Some(video_url) = &saved.recording_url {
            match trigger_assemblyai_transcription(db, config, &saved, video_url).await {
                Ok(_) => info!(
                    "AssemblyAI transcription triggered for recording {}",
                    saved.id
                ),
                Err(e) => warn!(
                    "Failed to trigger AssemblyAI for recording {}: {:?}",
                    saved.id, e
                ),
            }
        }
    }

    Some(saved)
}

/// Map Recall.ai status codes to our internal status
fn map_recall_status_code(code: &str) -> MeetingRecordingStatus {
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

/// Trigger AssemblyAI transcription for a completed recording
async fn trigger_assemblyai_transcription(
    db: &sea_orm::DatabaseConnection,
    config: &service::config::Config,
    recording: &MeetingRecordingModel,
    video_url: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use domain::gateway::assembly_ai::{create_standard_transcript_request, AssemblyAiClient};
    use domain::transcription as TranscriptionApi;
    use domain::transcription_status::TranscriptionStatus;

    // Check if transcription already exists for this recording
    if let Some(existing) = TranscriptionApi::find_by_meeting_recording_id(db, recording.id).await?
    {
        debug!(
            "Transcription {} already exists for recording {}",
            existing.id, recording.id
        );
        return Ok(());
    }

    // Get the coaching session to find the relationship
    let session = CoachingSessionApi::find_by_id(db, recording.coaching_session_id).await?;

    // Get the coaching relationship to find the coach
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Get the coach's user integrations
    let user_integrations = user_integration::find_by_user_id(db, relationship.coach_id)
        .await?
        .ok_or("Coach has no integrations configured")?;

    // Get the AssemblyAI API key
    let api_key = user_integrations
        .assembly_ai_api_key
        .as_ref()
        .ok_or("AssemblyAI API key not configured for coach")?;

    // Create a transcription record
    let mut transcription = TranscriptionApi::create(db, recording.id).await?;
    transcription.status = TranscriptionStatus::Processing;

    // Build the webhook URL for AssemblyAI callbacks
    let webhook_url = config
        .webhook_base_url()
        .map(|base| format!("{}/webhooks/assemblyai", base));
    let webhook_secret = config.webhook_secret().map(|s| s.to_string());

    // Create AssemblyAI client and send transcription request
    let client = AssemblyAiClient::new(api_key, config.assembly_ai_base_url())?;

    let request =
        create_standard_transcript_request(video_url.to_string(), webhook_url, webhook_secret);

    let response = client.create_transcript(request).await?;

    // Update transcription with AssemblyAI transcript ID
    transcription.assemblyai_transcript_id = Some(response.id.clone());
    TranscriptionApi::update(db, transcription.id, transcription).await?;

    info!(
        "Created AssemblyAI transcript {} for recording {}",
        response.id, recording.id
    );

    Ok(())
}
