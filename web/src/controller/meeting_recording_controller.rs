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
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET recording status for session: {session_id}");

    let recording: Option<MeetingRecordingModel> =
        MeetingRecordingApi::find_latest_by_coaching_session_id(
            app_state.db_conn_ref(),
            session_id,
        )
        .await?;

    match recording {
        Some(rec) => Ok(Json(ApiResponse::new(StatusCode::OK.into(), rec))),
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

    // 4. Check AI privacy level
    if relationship.ai_privacy_level == AiPrivacyLevel::None {
        return Err(bad_request_error(
            "AI recording is disabled for this coaching relationship",
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
