use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser,
    coaching_session_access::CoachingSessionAccess,
    coaching_session_topic_access::{
        CoachingSessionTopicAccess, CoachingSessionTopicAuthorAccess,
        CoachingSessionTopicCoacheeAccess, CoachingSessionTopicUndoAccess,
    },
    compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_session_topic as TopicApi;
use domain::Id;
use log::*;
use serde::Deserialize;
use service::config::ApiVersion;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateParams {
    pub body: String,
    /// Optional initial priority. Omit for new topics; null until the coachee triages.
    pub priority: Option<domain::topic_priority::Priority>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateParams {
    pub body: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReorderParams {
    pub ordered_ids: Vec<Id>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RatingParams {
    pub priority: Option<domain::topic_priority::Priority>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatusParams {
    pub status: domain::topic_status::Status,
}

/// GET all topics for a coaching session, in canonical order
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/topics",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Topics retrieved", body = [domain::coaching_session_topics::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching session not found"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET topics for session {}", session.id);

    let topics = TopicApi::find_by_coaching_session_id(app_state.db_conn_ref(), session.id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), topics)))
}

/// POST create a new topic on a coaching session
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/topics",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    request_body = CreateParams,
    responses(
        (status = 201, description = "Topic created", body = domain::coaching_session_topics::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching session not found"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(params): Json<CreateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST topic for session {}", session.id);

    let topic = TopicApi::create(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        session.id,
        params.body,
        user.id,
        params.priority,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), topic)))
}

/// PUT update a topic's body
#[utoipa::path(
    put,
    path = "/coaching_sessions/{coaching_session_id}/topics/{topic_id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("topic_id" = Id, Path, description = "Topic id"),
    ),
    request_body = UpdateParams,
    responses(
        (status = 200, description = "Topic updated", body = domain::coaching_session_topics::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Topic not found in this session"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionTopicAccess(topic): CoachingSessionTopicAccess,
    State(app_state): State<AppState>,
    Json(params): Json<UpdateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT topic {}", topic.id);

    let updated = TopicApi::update(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        topic.id,
        params.body,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}

/// PATCH reorder all topics in a coaching session
#[utoipa::path(
    patch,
    path = "/coaching_sessions/{coaching_session_id}/topics/reorder",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    request_body = ReorderParams,
    responses(
        (status = 200, description = "Topics reordered", body = [domain::coaching_session_topics::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching session not found"),
        (status = 422, description = "Provided ids are not a permutation of the session's topics"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn reorder(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionAccess(session): CoachingSessionAccess,
    State(app_state): State<AppState>,
    Json(params): Json<ReorderParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("PATCH reorder topics for session {}", session.id);

    let topics = TopicApi::reorder(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        session.id,
        params.ordered_ids,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), topics)))
}

/// DELETE a topic (author only)
#[utoipa::path(
    delete,
    path = "/coaching_sessions/{coaching_session_id}/topics/{topic_id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("topic_id" = Id, Path, description = "Topic id"),
    ),
    responses(
        (status = 200, description = "Topic deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Topic not found in this session or not authored by the caller"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionTopicAuthorAccess(topic): CoachingSessionTopicAuthorAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("DELETE topic {}", topic.id);

    TopicApi::delete(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        topic.id,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::json!({ "id": topic.id }),
    )))
}

/// PATCH set a topic's priority (coachee only)
#[utoipa::path(
    patch,
    path = "/coaching_sessions/{coaching_session_id}/topics/{topic_id}/rating",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("topic_id" = Id, Path, description = "Topic id"),
    ),
    request_body = RatingParams,
    responses(
        (status = 200, description = "Topic priority updated", body = domain::coaching_session_topics::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Only the coachee may set a topic's priority"),
        (status = 404, description = "Topic not found in this session"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn set_rating(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionTopicCoacheeAccess(topic): CoachingSessionTopicCoacheeAccess,
    State(app_state): State<AppState>,
    Json(params): Json<RatingParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("PATCH priority for topic {}", topic.id);

    let updated = TopicApi::set_priority(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        topic.id,
        params.priority,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}

/// PATCH set a topic's lifecycle status (either participant)
#[utoipa::path(
    patch,
    path = "/coaching_sessions/{coaching_session_id}/topics/{topic_id}/status",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("topic_id" = Id, Path, description = "Topic id"),
    ),
    request_body = StatusParams,
    responses(
        (status = 200, description = "Topic status updated", body = domain::coaching_session_topics::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Topic not found in this session"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn set_status(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionTopicAccess(topic): CoachingSessionTopicAccess,
    State(app_state): State<AppState>,
    Json(params): Json<StatusParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("PATCH status for topic {}", topic.id);

    let updated = TopicApi::set_status(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        topic.id,
        params.status,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}

/// Undo the most recent reversible change to a topic (a deferral or a deletion).
///
/// Restores the topic to the state it held immediately before that change: its
/// original session, status, priority, position, and timestamps are all brought
/// back, so the reversal is a faithful no-op rather than a fresh write. The server
/// decides what to reverse from the topic's own state, not from any request input:
/// a deleted topic is un-deleted in place, a deferred or moved topic is returned to
/// the session it came from, and a topic with nothing pending yields 422. Takes no
/// request body. The caller must be a participant of the topic's session; undoing a
/// deletion additionally requires that the caller authored the topic.
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/topics/{topic_id}/undo",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
        ("topic_id" = Id, Path, description = "Topic id"),
    ),
    responses(
        (status = 200, description = "Change undone; the restored topic is returned", body = domain::coaching_session_topics::Model),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Topic not found in this session, caller is not a participant, or (when undoing a deletion) not the author"),
        (status = 422, description = "Topic has no reversible change to undo"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn undo(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingSessionTopicUndoAccess(topic): CoachingSessionTopicUndoAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST undo for topic {}", topic.id);

    let updated = TopicApi::undo(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        topic.id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated)))
}
