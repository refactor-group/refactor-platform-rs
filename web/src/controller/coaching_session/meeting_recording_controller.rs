use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::meeting_recording as MeetingRecordingApi;
use domain::meeting_recording::MeetingRecordingStatus;
use domain::Id;
use log::*;
use serde::Deserialize;
use service::config::ApiVersion;

#[derive(Debug, Deserialize)]
pub struct StartRecordingParams {
    pub meeting_url: String,
}

/// GET the current recording status and artifact URLs for a coaching session
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Recording status retrieved"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET meeting_recording for session {}", coaching_session_id);

    let recording = MeetingRecordingApi::find_latest_by_coaching_session(
        app_state.db_conn_ref(),
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), recording)))
}

/// POST create a Recall.ai bot and start recording a coaching session
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    request_body = StartRecordingParams,
    responses(
        (status = 201, description = "Recording bot created and joined meeting"),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "An active recording already exists for this session"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
    Json(params): Json<StartRecordingParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST meeting_recording for session {}", coaching_session_id);

    // Prevent duplicate active bots
    if let Some(existing) = MeetingRecordingApi::find_latest_by_coaching_session(
        app_state.db_conn_ref(),
        coaching_session_id,
    )
    .await?
    {
        let active = !matches!(
            existing.status,
            MeetingRecordingStatus::Failed | MeetingRecordingStatus::Completed
        );
        if active {
            warn!(
                "Active recording {} already exists for session {}",
                existing.id, coaching_session_id
            );
            return Err(Error::Web(WebErrorKind::Conflict));
        }
    }

    let recording = MeetingRecordingApi::start(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session_id,
        &params.meeting_url,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        recording,
    )))
}

/// DELETE stop the active recording bot for a coaching session
#[utoipa::path(
    delete,
    path = "/coaching_sessions/{coaching_session_id}/meeting_recording",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Recording bot stopped"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "No active recording found for this session"),
        (status = 503, description = "Service temporarily unavailable"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "DELETE meeting_recording for session {}",
        coaching_session_id
    );

    let recording = MeetingRecordingApi::stop(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), recording)))
}
