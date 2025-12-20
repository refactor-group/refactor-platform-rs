//! Controller for transcript and transcription operations.
//!
//! Handles retrieval of transcripts and transcript segments for coaching sessions.

use crate::controller::ApiResponse;
use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use domain::coaching_relationship as CoachingRelationshipApi;
use domain::coaching_session as CoachingSessionApi;
use domain::meeting_recording as MeetingRecordingApi;
use domain::transcript_segment;
use domain::transcript_segments::Model as TranscriptSegmentModel;
use domain::transcription as TranscriptionApi;
use domain::transcriptions::Model as TranscriptionModel;
use domain::Id;
use log::*;
use service::config::ApiVersion;

/// GET /coaching_sessions/{id}/transcript
///
/// Get the transcription for a coaching session.
/// Returns the transcript with summary, full text, and metadata.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/transcript",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Transcription retrieved", body = transcriptions::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No transcription found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_transcript(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the transcript
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view transcript for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this transcript".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), transcription)))
}

/// GET /coaching_sessions/{id}/transcript/segments
///
/// Get the transcript segments (utterances) for a coaching session.
/// Returns speaker-labeled segments with timestamps.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/transcript/segments",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Transcript segments retrieved", body = Vec<transcript_segments::Model>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No transcript found for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_transcript_segments(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET transcript segments for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the transcript
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view transcript segments for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this transcript".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 5. Get the segments for this transcription
    let segments: Vec<TranscriptSegmentModel> =
        transcript_segment::find_by_transcription_id(db, transcription.id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), segments)))
}

/// GET /coaching_sessions/{id}/summary
///
/// Get just the AI-generated summary for a coaching session.
/// This is a convenience endpoint that returns only the summary text.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{id}/summary",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching session ID"),
    ),
    responses(
        (status = 200, description = "Summary retrieved", body = SummaryResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - not a participant in this session"),
        (status = 404, description = "No summary available for this session"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn get_session_summary(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET summary for session: {session_id}");

    let db = app_state.db_conn_ref();

    // 1. Get the coaching session
    let session = CoachingSessionApi::find_by_id(db, session_id).await?;

    // 2. Get the coaching relationship and verify access
    let relationship =
        CoachingRelationshipApi::find_by_id(db, session.coaching_relationship_id).await?;

    // Only coach or coachee can view the summary
    if relationship.coach_id != user.id && relationship.coachee_id != user.id {
        warn!(
            "User {} attempted to view summary for session {} but is not a participant",
            user.id, session_id
        );
        return Err(Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                    "Not authorized to view this summary".to_string(),
                )),
            ),
        }));
    }

    // 3. Get the latest recording for this session
    let recording = MeetingRecordingApi::find_latest_by_coaching_session_id(db, session_id)
        .await?
        .ok_or_else(|| {
            Error::Domain(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::NotFound,
                    ),
                ),
            })
        })?;

    // 4. Get the transcription for this recording
    let transcription: TranscriptionModel =
        TranscriptionApi::find_by_meeting_recording_id(db, recording.id)
            .await?
            .ok_or_else(|| {
                Error::Domain(domain::error::Error {
                    source: None,
                    error_kind: domain::error::DomainErrorKind::Internal(
                        domain::error::InternalErrorKind::Entity(
                            domain::error::EntityErrorKind::NotFound,
                        ),
                    ),
                })
            })?;

    // 5. Check if we have a summary
    let summary = transcription.summary.ok_or_else(|| {
        Error::Domain(domain::error::Error {
            source: None,
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::NotFound),
            ),
        })
    })?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SummaryResponse {
            session_id,
            summary,
            word_count: transcription.word_count,
            confidence_score: transcription.confidence_score,
        },
    )))
}

/// Response for the summary endpoint
#[derive(Debug, serde::Serialize)]
pub struct SummaryResponse {
    pub session_id: Id,
    pub summary: String,
    pub word_count: Option<i32>,
    pub confidence_score: Option<f64>,
}
