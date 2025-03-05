use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::{IndexParams, UpdateParams};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_session as CoachingSessionApi, coaching_sessions::Model, Id};
use service::config::ApiVersion;

use log::*;

#[utoipa::path(
    get,
    path = "/coaching_sessions",
    params(
        ApiVersion,
        ("coaching_relationship_id" = Option<Id>, Query, description = "Filter by coaching_relationship_id"),
        ("from_date" = Option<NaiveDate>, Query, description = "Filter by from_date"),
        ("to_date" = Option<NaiveDate>, Query, description = "Filter by to_date")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Coaching Sessions", body = [coaching_sessions::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Coaching Sessions");
    debug!("Filter Params: {:?}", params);

    let coaching_sessions = CoachingSessionApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Coaching Sessions: {:?}", coaching_sessions);

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_sessions,
    )))
}

/// POST create a new Coaching Session
#[utoipa::path(
    post,
    path = "/coaching_sessions",
    params(ApiVersion),
    request_body = coaching_sessions::Model,
    responses(
        (status = 201, description = "Successfully Created a new Coaching Session", body = [coaching_sessions::Model]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(coaching_sessions_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "POST Create a new Coaching Session from: {:?}",
        coaching_sessions_model
    );

    let coaching_session = CoachingSessionApi::create(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_sessions_model,
    )
    .await?;

    debug!("New Coaching Session: {:?}", coaching_session);

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_session,
    )))
}

/// PUT update a Coaching Session
#[utoipa::path(
    put,
    path = "/coaching_sessions/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Coaching Session ID to Update")
    ),
    request_body = UpdateParams,
    responses(
        (status = 204, description = "Successfully updated a Coaching Session", body = ()),
        (status = 401, description = "Unauthorized"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionApi::update(app_state.db_conn_ref(), coaching_session_id, params).await?;
    Ok(Json(ApiResponse::new(StatusCode::NO_CONTENT.into(), ())))
}
