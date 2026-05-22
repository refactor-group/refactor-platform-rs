use crate::controller::ApiResponse;
use crate::extractors::{
    coaching_session_access::CoachingSessionAccess, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::transcription as TranscriptionApi;
use log::*;
use service::config::ApiVersion;

/// GET transcription metadata and status for a coaching session
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/transcriptions",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Transcription metadata retrieved"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let coaching_session_id = session.id;
    debug!("GET transcription for session {}", coaching_session_id);

    let transcription =
        TranscriptionApi::find_by_coaching_session(app_state.db_conn_ref(), coaching_session_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), transcription)))
}
