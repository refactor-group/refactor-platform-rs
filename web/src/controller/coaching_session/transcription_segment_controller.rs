use crate::controller::ApiResponse;
use crate::extractors::{
    coaching_session_access::CoachingSessionAccess, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::transcription as TranscriptionApi;
use domain::Id;
use log::*;
use service::config::ApiVersion;

/// GET a transcription's segments in speaking order, each labeled with who spoke it.
///
/// `speaker_label` is the profile name for the session's coach or coachee, the name the
/// speaker typed in the meeting otherwise, or `Guest N` when there is none. Labels are unique
/// per transcript and identical to the `speakers` labels of the transcription read.
/// `speaker_user_id` and `speaker_role` identify the coach or coachee, and are `null` for
/// anyone else. Empty when the transcription is not under this session or has no segments.
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions/{transcription_id}/transcription_segments",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Path, description = "Coaching session id"),
        ("transcription_id" = Uuid, Path, description = "Transcription id"),
    ),
    responses(
        (status = 200, description = "Labeled transcript segments ordered by start time", body = [domain::transcript_export::LabeledSegment]),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Path((_coaching_session_id, transcription_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET transcription_segments for transcription {}",
        transcription_id
    );

    let segments =
        TranscriptionApi::read_segments(app_state.db_conn_ref(), &session, transcription_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), segments)))
}
