use crate::controller::ApiResponse;
use crate::extractors::coaching_session_series_access::{
    CoachingRelationshipQueryAccess, CoachingSessionSeriesAccess, CoachingSessionSeriesCoachAccess,
};
use crate::extractors::{
    authenticated_user::AuthenticatedUser,
    coaching_relationship_access::authorize_relationship_coach,
    compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session_series::{
    CreateParams, RescheduleFollowingParams, RescheduleParams,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::{
    coaching_session as CoachingSessionApi, coaching_session_series as CoachingSessionSeriesApi,
};
use serde::Serialize;
use service::config::ApiVersion;
use utoipa::ToSchema;

use log::*;

/// Response body for series-create and series-read endpoints.
#[derive(Debug, Serialize, ToSchema)]
pub struct SeriesWithSessions {
    pub series: CoachingSessionSeriesApi::Model,
    pub sessions: Vec<domain::coaching_sessions::Model>,
}

/// `POST /coaching_session_series` — create a recurring series and
/// materialize its sessions in one transaction.
#[utoipa::path(
    post,
    path = "/coaching_session_series",
    params(ApiVersion),
    request_body = CreateParams,
    responses(
        (status = 201, description = "Series created", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not the coach on the target relationship"),
        (status = 404, description = "Coaching relationship not found"),
        (status = 409, description = "The coachee is no longer a member of the organization; `details.reason` is `coachee_not_a_member` (with `details.user_id` and `details.organization_id`)"),
        (status = 422, description = "Invalid recurrence rule"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateParams>,
) -> Result<Response, Error> {
    debug!("POST create coaching_session_series: {params:?}");

    if let Err(rejection) =
        authorize_relationship_coach(&app_state, &user, params.coaching_relationship_id).await
    {
        return Ok(rejection.into_response());
    }

    let db = app_state.db_conn_ref();

    let requested_duration = CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?;

    let (series, sessions) = CoachingSessionSeriesApi::create_with_sessions(
        db,
        &app_state.config,
        params.coaching_relationship_id,
        user.id,
        params.start_at,
        params.recurrence,
        requested_duration,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        SeriesWithSessions { series, sessions },
    ))
    .into_response())
}

/// `GET /coaching_session_series/:id` — read one series with its sessions.
#[utoipa::path(
    get,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Series ID")
    ),
    responses(
        (status = 200, description = "Series", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not a participant of the relationship"),
        (status = 404, description = "Not found")
    ),
    security(("cookie_auth" = []))
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesAccess(series): CoachingSessionSeriesAccess,
) -> Result<impl IntoResponse, Error> {
    // The extractor already loaded the series row and validated access; do a
    // separate read for the linked sessions, ordered by date asc.
    let sessions =
        CoachingSessionApi::find_by_series_id(app_state.db_conn_ref(), series.id).await?;
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SeriesWithSessions { series, sessions },
    )))
}

/// `GET /coaching_session_series?coaching_relationship_id=...` — list series
/// metadata for a relationship (no nested sessions).
#[utoipa::path(
    get,
    path = "/coaching_session_series",
    params(
        ApiVersion,
        ("coaching_relationship_id" = Uuid, Query, description = "Filter by coaching relationship")
    ),
    responses(
        (status = 200, description = "Series list", body = [CoachingSessionSeriesApi::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not a participant of the relationship")
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingRelationshipQueryAccess(relationship): CoachingRelationshipQueryAccess,
) -> Result<impl IntoResponse, Error> {
    let series =
        CoachingSessionSeriesApi::find_by_relationship(app_state.db_conn_ref(), relationship.id)
            .await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), series)))
}

/// `PUT /coaching_session_series/:id` — reschedule. Replaces the rule and
/// re-materializes future sessions; past sessions stay put.
#[utoipa::path(
    put,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Series ID")
    ),
    request_body = RescheduleParams,
    responses(
        (status = 200, description = "Rescheduled", body = SeriesWithSessions),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not the coach on the relationship"),
        (status = 409, description = "The coachee is no longer a member of the organization; `details.reason` is `coachee_not_a_member` (with `details.user_id` and `details.organization_id`)"),
        (status = 422, description = "Invalid recurrence rule")
    ),
    security(("cookie_auth" = []))
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
    Json(params): Json<RescheduleParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "PUT reschedule coaching_session_series id={} params={params:?}",
        series.id
    );

    let db = app_state.db_conn_ref();
    let requested_duration = CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?;

    let rescheduled = CoachingSessionSeriesApi::reschedule(
        db,
        &app_state.config,
        series.id,
        user.id,
        params.start_at,
        params.recurrence,
        requested_duration,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SeriesWithSessions {
            series: rescheduled.series,
            sessions: rescheduled.sessions,
        },
    )))
}

/// Reschedule one session of a series and every session after it.
///
/// `POST /coaching_session_series/:id/reschedule`. The sessions move in place and keep their
/// ids and content. A session moved or resized on its own keeps that date or length; the
/// chosen session always takes the request's. When earlier sessions stay behind, the series
/// splits and `series` in the response is the new one holding the moved sessions.
#[utoipa::path(
    post,
    path = "/coaching_session_series/{id}/reschedule",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Series ID")
    ),
    request_body = RescheduleFollowingParams,
    responses(
        (status = 200, description = "Rescheduled; `series` holds the moved sessions", body = SeriesWithSessions),
        (status = 400, description = "Missing or unsupported API version header (plain text)"),
        (status = 401, description = "Unauthorized (plain text)"),
        (status = 403, description = "Forbidden: not the coach on the relationship (plain text)"),
        (status = 404, description = "Not found (plain text)"),
        (status = 409, description = "Leftover sessions have content. JSON `{status_code, error: \"conflict\", message, details: {reason: \"sessions_have_content\", coaching_session_ids}}`, or the coachee is no longer a member of the organization: `details: {reason: \"coachee_not_a_member\", user_id, organization_id}`"),
        (status = 422, description = "Invalid session, start, duration or recurrence rule. JSON `{status_code, error: \"validation_error\", message}`, or plain text for a malformed body")
    ),
    security(("cookie_auth" = []))
)]
pub async fn reschedule_following(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
    Json(params): Json<RescheduleFollowingParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "POST reschedule following coaching_session_series id={} params={params:?}",
        series.id
    );

    let rescheduled = CoachingSessionSeriesApi::reschedule_following(
        app_state.db_conn_ref(),
        &app_state.config,
        series.id,
        user.id,
        CoachingSessionSeriesApi::FollowingReschedule {
            coaching_session_id: params.coaching_session_id,
            start_at: params.start_at,
            duration: CoachingSessionApi::parse_duration_minutes(params.duration_minutes)?,
            recurrence: params.recurrence,
        },
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        SeriesWithSessions {
            series: rescheduled.series,
            sessions: rescheduled.sessions,
        },
    )))
}

/// `DELETE /coaching_session_series/:id` — delete series + future sessions.
/// Past sessions survive as orphan one-offs (FK SET NULL).
#[utoipa::path(
    delete,
    path = "/coaching_session_series/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Series ID")
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not the coach on the relationship")
    ),
    security(("cookie_auth" = []))
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionSeriesApi::delete_with_future_sessions(
        app_state.db_conn_ref(),
        &app_state.config,
        series.id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /coaching_session_series/:id/resend_invite` — resend the series invite to both
/// participants as it stands today. Synchronous, so the answer says whether it went.
#[utoipa::path(
    post,
    path = "/coaching_session_series/{id}/resend_invite",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Series ID")
    ),
    responses(
        (status = 204, description = "Invite resent"),
        (status = 400, description = "Missing or unsupported API version header"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — not the coach on the relationship"),
        (status = 404, description = "Not found"),
        (status = 409, description = "Not resendable; `details.reason` is `no_upcoming_sessions`, `legacy_series`, or `coachee_not_a_member` (with `details.user_id` and `details.organization_id`)"),
        (status = 502, description = "The invite did not reach the coachee")
    ),
    security(("cookie_auth" = []))
)]
pub async fn resend_invite(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    CoachingSessionSeriesCoachAccess(series): CoachingSessionSeriesCoachAccess,
) -> Result<impl IntoResponse, Error> {
    CoachingSessionSeriesApi::resend_invite(app_state.db_conn_ref(), &app_state.config, series.id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "coaching_session_series_controller_sqlite_tests.rs"]
mod sqlite_tests;
