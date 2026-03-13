use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_sessions_goals::Model;
use domain::goal as GoalApi;
use domain::Id;
use service::config::ApiVersion;

use log::*;

/// POST link a goal to a coaching session
#[utoipa::path(
    post,
    path = "/coaching_sessions/{coaching_session_id}/goals",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
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
    Path(coaching_session_id): Path<Id>,
    Json(model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "POST Link goal {} to session {}",
        model.goal_id, coaching_session_id
    );

    let link = GoalApi::link_to_coaching_session(
        app_state.db_conn_ref(),
        coaching_session_id,
        model.goal_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), link)))
}

/// DELETE unlink a goal from a coaching session
#[utoipa::path(
    delete,
    path = "/coaching_sessions/{coaching_session_id}/goals/{id}",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
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
    Path((_coaching_session_id, id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!("DELETE coaching_session_goal link by id: {id}");

    GoalApi::unlink_from_coaching_session(app_state.db_conn_ref(), id).await?;

    Ok(Json(serde_json::json!({"id": id})))
}

/// GET goals linked to a specific coaching session (eager-loaded)
#[utoipa::path(
    get,
    path = "/coaching_sessions/{coaching_session_id}/goals",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Path, description = "Coaching session id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goals linked to session", body = [entity::goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(coaching_session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET goals linked to session {coaching_session_id}");

    let goals =
        GoalApi::find_goals_by_coaching_session_id(app_state.db_conn_ref(), coaching_session_id)
            .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), goals)))
}
