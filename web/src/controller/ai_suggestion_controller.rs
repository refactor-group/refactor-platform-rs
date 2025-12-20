//! Controller for AI suggestion operations.
//!
//! Handles retrieving, accepting, and dismissing AI-suggested actions and agreements.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::actions::Model as ActionModel;
use domain::agreements::Model as AgreementModel;
use domain::ai_suggested_item as AiSuggestionApi;
use domain::ai_suggested_items::Model as AiSuggestionModel;
use domain::ai_suggestion::{AiSuggestionStatus, AiSuggestionType};
use domain::coaching_relationship as CoachingRelationshipApi;
use domain::coaching_session as CoachingSessionApi;
use domain::meeting_recording as MeetingRecordingApi;
use domain::status::Status;
use domain::transcription as TranscriptionApi;
use domain::{action as ActionApi, agreement as AgreementApi, Id};
use log::*;
use service::config::ApiVersion;

/// GET /coaching_sessions/{id}/ai-suggestions
///
/// Get pending AI suggestions for a coaching session.
/// Returns actions and agreements detected by AI from the transcript.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/ai-suggestions",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "AI suggestions retrieved", body = Vec<ai_suggested_items::Model>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No transcription found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_session_suggestions(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET AI suggestions for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view suggestions
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view AI suggestions for session {} but is not a participant",
            user.id, session_id
        );
        return Err(forbidden_error("Not authorized to view these suggestions"));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| not_found_error("No recording found for this session"))?;

    // 4. Get the transcription for this recording
    let transcription = TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
        .await?
        .ok_or_else(|| not_found_error("No transcription found for this session"))?;

    // 5. Get pending suggestions for this transcription
    let suggestions: Vec<AiSuggestionModel> =
        AiSuggestionApi::find_pending_by_transcription_id(db, transcription.id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), suggestions)))
}

/// POST /ai-suggestions/{id}/accept
///
/// Accept an AI suggestion and create the corresponding Action or Agreement.
/// The suggestion will be linked to the newly created entity.
#[utoipa::path(
    post,
    path = "/ai-suggestions/{id}/accept",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "AI suggestion ID"),
    ),
    responses(
        (status = 201, description = "Suggestion accepted and entity created", body = AcceptResponse),
        (status = 400, description = "Suggestion already processed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "Suggestion not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn accept_suggestion(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(suggestion_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("Accepting AI suggestion: {suggestion_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the suggestion
    let suggestion = AiSuggestionApi::find_by_id(db, suggestion_id).await?;

    // 2. Verify suggestion is still pending
    if suggestion.status != AiSuggestionStatus::Pending {
        return Err(bad_request_error("Suggestion has already been processed"));
    }

    // 3. Get the transcription to find the coaching session
    let transcription = TranscriptionApi::find_by_id(db, suggestion.transcription_id).await?;

    // 4. Get the meeting recording
    let recording = MeetingRecordingApi::find_by_id(db, transcription.meeting_recording_id).await?;

    // 5. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, recording.coaching_session_id).await?;

    // 6. Verify user has access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to accept AI suggestion {} but is not a participant",
            user.id, suggestion_id
        );
        return Err(forbidden_error("Not authorized to accept this suggestion"));
    }

    // 7. Create the entity based on suggestion type
    let (entity_id, entity_type) = match suggestion.item_type {
        AiSuggestionType::Action => {
            let action_model = ActionModel {
                id: Id::default(),
                coaching_session_id: session.id,
                user_id: user.id,
                body: Some(suggestion.content.clone()),
                due_by: None,
                status: Status::NotStarted,
                status_changed_at: chrono::Utc::now().into(),
                created_at: chrono::Utc::now().into(),
                updated_at: chrono::Utc::now().into(),
            };

            let created_action: ActionModel = ActionApi::create(db, action_model, user.id).await?;
            info!("Created action {} from AI suggestion", created_action.id);
            (created_action.id, "action")
        }
        AiSuggestionType::Agreement => {
            let agreement_model = AgreementModel {
                id: Id::default(),
                coaching_session_id: session.id,
                body: Some(suggestion.content.clone()),
                user_id: user.id,
                created_at: chrono::Utc::now().into(),
                updated_at: chrono::Utc::now().into(),
            };

            let created_agreement: AgreementModel =
                AgreementApi::create(db, agreement_model, user.id).await?;
            info!(
                "Created agreement {} from AI suggestion",
                created_agreement.id
            );
            (created_agreement.id, "agreement")
        }
    };

    // 8. Update the suggestion as accepted
    let updated_suggestion: AiSuggestionModel =
        AiSuggestionApi::accept(db, suggestion_id, entity_id).await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiResponse::new(
            StatusCode::CREATED.into(),
            AcceptResponse {
                suggestion: updated_suggestion,
                entity_id,
                entity_type: entity_type.to_string(),
            },
        )),
    ))
}

/// POST /ai-suggestions/{id}/dismiss
///
/// Dismiss an AI suggestion. The suggestion will be marked as dismissed
/// and will no longer appear in the pending list.
#[utoipa::path(
    post,
    path = "/ai-suggestions/{id}/dismiss",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "AI suggestion ID"),
    ),
    responses(
        (status = 200, description = "Suggestion dismissed", body = ai_suggested_items::Model),
        (status = 400, description = "Suggestion already processed"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "Suggestion not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn dismiss_suggestion(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(suggestion_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    info!("Dismissing AI suggestion: {suggestion_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the suggestion
    let suggestion = AiSuggestionApi::find_by_id(db, suggestion_id).await?;

    // 2. Verify suggestion is still pending
    if suggestion.status != AiSuggestionStatus::Pending {
        return Err(bad_request_error("Suggestion has already been processed"));
    }

    // 3. Get the transcription to find the coaching session
    let transcription = TranscriptionApi::find_by_id(db, suggestion.transcription_id).await?;

    // 4. Get the meeting recording
    let recording = MeetingRecordingApi::find_by_id(db, transcription.meeting_recording_id).await?;

    // 5. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, recording.coaching_session_id).await?;

    // 6. Verify user has access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to dismiss AI suggestion {} but is not a participant",
            user.id, suggestion_id
        );
        return Err(forbidden_error("Not authorized to dismiss this suggestion"));
    }

    // 7. Dismiss the suggestion
    let updated_suggestion: AiSuggestionModel = AiSuggestionApi::dismiss(db, suggestion_id).await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        updated_suggestion,
    )))
}

/// Response for accepting a suggestion
#[derive(Debug, serde::Serialize)]
pub struct AcceptResponse {
    pub suggestion: AiSuggestionModel,
    pub entity_id: Id,
    pub entity_type: String,
}

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

/// Helper to create a not found error
fn not_found_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}
