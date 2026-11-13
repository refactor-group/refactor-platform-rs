use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser,
    coaching_relationship_access::{authorize_relationship, CoachingRelationshipQueryAccess},
    compare_api_version::CompareApiVersion,
    goal_access::GoalAccess,
};
use crate::params::goal::{IndexParams, SortField};
use crate::params::status;
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::coaching_session as CoachingSessionApi;
use domain::goal as GoalApi;
use domain::goal_progress as GoalProgressApi;
use domain::goals::Model;
use serde_json::json;
use service::config::ApiVersion;

use log::*;

/// GET sessions linked to a specific goal
#[utoipa::path(
    get,
    path = "/goals/{id}/sessions",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Goal id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved sessions linked to goal", body = [domain::coaching_sessions_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn coaching_sessions_by_goal(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = goal.id;
    debug!("GET sessions linked to goal {id}");

    let links = GoalApi::find_coaching_sessions_by_goal_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), links)))
}

/// POST create a new Goal
#[utoipa::path(
    post,
    path = "/goals",
    params(ApiVersion),
    request_body = domain::goals::Model,
    responses(
        (status = 201, description = "Successfully Created a New Goal", body = domain::goals::Model),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Coaching relationship or created-in session not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(goal_model): Json<Model>,
) -> Result<Response, Error> {
    debug!("POST Create a New Goal from: {goal_model:?}");

    if let Err(rejection) =
        authorize_relationship(&app_state, &user, goal_model.coaching_relationship_id).await
    {
        return Ok(rejection.into_response());
    }

    if let Some(session_id) = goal_model.created_in_session_id {
        let session = CoachingSessionApi::find_by_id(app_state.db_conn_ref(), session_id).await?;
        if session.coaching_relationship_id != goal_model.coaching_relationship_id {
            return Err(Error::validation(
                "created_in_session_id must name a session of the goal's relationship.",
            ));
        }
    }

    let goal = GoalApi::create(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        goal_model,
        user.id,
    )
    .await?;

    debug!("New Goal: {goal:?}");

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), goal)).into_response())
}

/// GET a particular Goal specified by its id.
#[utoipa::path(
    get,
    path = "/goals/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Goal id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Goal by its id", body = domain::goals::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Goal not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Goal by id: {}", goal.id);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), goal)))
}

#[utoipa::path(
    put,
    path = "/goals/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of goal to update"),
    ),
    request_body = domain::goals::Model,
    responses(
        (status = 200, description = "Successfully Updated Goal", body = domain::goals::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(goal_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    let id = goal.id;
    debug!("PUT Update Goal with id: {id}");

    let goal = GoalApi::update(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        goal_model,
    )
    .await?;

    debug!("Updated Goal: {goal:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), goal)))
}

#[utoipa::path(
    put,
    path = "/goals/{id}/status",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of goal to update"),
        ("value" = String, Query, description = "New status: not_started, in_progress, completed, on_hold or wont_do"),
    ),
    responses(
        (status = 200, description = "Successfully Updated Goal", body = domain::goals::Model),
        (status = 400, description = "Missing value"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Goal not found"),
        (status = 405, description = "Method not allowed"),
        (status = 422, description = "Unknown status value"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_status(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    Query(params): Query<status::UpdateParams>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = goal.id;
    debug!("PUT Update Goal Status with id: {id}");

    let goal = GoalApi::update_status(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        params.status()?,
    )
    .await?;

    debug!("Updated Goal: {goal:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), goal)))
}

/// DELETE a Goal by id
#[utoipa::path(
    delete,
    path = "/goals/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of goal to delete"),
    ),
    responses(
        (status = 200, description = "Successfully Deleted Goal"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Goal not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = goal.id;
    debug!("DELETE Goal by id: {id}");

    GoalApi::delete(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
    )
    .await?;

    Ok(Json(json!({"id": id})))
}

#[utoipa::path(
    get,
    path = "/goals",
    params(
        ApiVersion,
        ("coaching_relationship_id" = Uuid, Query, description = "Filter by coaching_relationship_id"),
        ("status" = Option<domain::status::Status>, Query, description = "Filter by status (e.g., 'InProgress', 'Completed')"),
        ("sort_by" = Option<crate::params::goal::SortField>, Query, description = "Sort by field. Valid values: 'title', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "title"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Goals", body = [domain::goals::Model]),
        (status = 400, description = "Invalid query parameter value"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CoachingRelationshipQueryAccess(_relationship): CoachingRelationshipQueryAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Goals");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(
        &mut params.sort_by,
        &mut params.sort_order,
        SortField::Title,
    );

    let goals = GoalApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Goals: {goals:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), goals)))
}

/// GET progress metrics for a specific goal
#[utoipa::path(
    get,
    path = "/goals/{id}/progress",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Goal id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goal progress metrics"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Goal not found"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn progress(
    GoalAccess { goal }: GoalAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = goal.id;
    debug!("GET Goal progress metrics for id: {id}");

    let metrics = GoalProgressApi::progress_metrics(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), metrics)))
}

#[cfg(test)]
#[path = "goal_controller_sqlite_tests.rs"]
mod sqlite_tests;
