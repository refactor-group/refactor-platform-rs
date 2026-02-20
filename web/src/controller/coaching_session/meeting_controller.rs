use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::meeting::CreateParams;
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_session as CoachingSessionApi, Id};
use log::*;
use service::config::ApiVersion;

/// POST create a meeting for a Coaching Session
#[utoipa::path(
    post,
    path = "/coaching_sessions/{id}/meetings",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching Session ID"),
    ),
    request_body = CreateParams,
    responses(
        (status = 201, description = "Meeting created successfully", body = domain::coaching_sessions::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the coach can create meetings"),
        (status = 404, description = "Coaching session not found"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
    Json(params): Json<CreateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create meeting for coaching session: {session_id}");

    let coaching_session = CoachingSessionApi::create_meeting(
        app_state.db_conn_ref(),
        &app_state.config,
        session_id,
        params.provider,
    )
    .await?;

    debug!("Meeting created for coaching session: {coaching_session:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_session,
    )))
}
