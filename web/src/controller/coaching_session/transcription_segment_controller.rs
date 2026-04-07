use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::transcript_segment as TranscriptionSegmentApi;
use domain::Id;
use log::*;
use service::config::ApiVersion;

/// GET ordered transcript segments for a transcription (powers the conversation UI)
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions/{transcription_id}/transcription_segments",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("transcription_id" = Id, Path, description = "Transcription id"),
    ),
    responses(
        (status = 200, description = "Transcript segments retrieved ordered by start time"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path((_coaching_session_id, transcription_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET transcription_segments for transcription {}",
        transcription_id
    );

    let segments =
        TranscriptionSegmentApi::find_by_transcription(app_state.db_conn_ref(), transcription_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), segments)))
}
