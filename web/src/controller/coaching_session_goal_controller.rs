use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_session_goal as CoachingSessionGoalApi;
use domain::coaching_sessions_goals::Model;
use domain::Id;
use serde_json::json;
use service::config::ApiVersion;

use log::*;

/// POST link a goal to a coaching session
#[utoipa::path(
    post,
    path = "/coaching_session_goals",
    params(ApiVersion),
    request_body = entity::coaching_sessions_goals::Model,
    responses(
        (status = 201, description = "Successfully linked goal to session", body = [entity::coaching_sessions_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 409, description = "Goal already linked to session"),
        (status = 422, description = "Unprocessable Entity"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "POST Link goal {} to session {}",
        model.goal_id, model.coaching_session_id
    );

    let link = CoachingSessionGoalApi::create(
        app_state.db_conn_ref(),
        model.coaching_session_id,
        model.goal_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), link)))
}

/// DELETE unlink a goal from a coaching session
#[utoipa::path(
    delete,
    path = "/coaching_session_goals/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of the coaching_session_goal link to remove"),
    ),
    responses(
        (status = 200, description = "Successfully unlinked goal from session"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Link not found"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("DELETE coaching_session_goal link by id: {id}");

    CoachingSessionGoalApi::delete_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(json!({"id": id})))
}

/// GET goals linked to a specific coaching session
#[utoipa::path(
    get,
    path = "/coaching_sessions/{session_id}/goals",
    params(
        ApiVersion,
        ("session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goals linked to session", body = [entity::coaching_sessions_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn goals_by_session(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET goals linked to session {session_id}");

    let links =
        CoachingSessionGoalApi::find_by_session_id(app_state.db_conn_ref(), session_id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), links)))
}

/// GET sessions linked to a specific goal
#[utoipa::path(
    get,
    path = "/goals/{goal_id}/sessions",
    params(
        ApiVersion,
        ("goal_id" = Id, Path, description = "Goal id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved sessions linked to goal", body = [entity::coaching_sessions_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn sessions_by_goal(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(goal_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET sessions linked to goal {goal_id}");

    let links = CoachingSessionGoalApi::find_by_goal_id(app_state.db_conn_ref(), goal_id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), links)))
}
