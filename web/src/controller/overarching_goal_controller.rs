use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::overarching_goal::{IndexParams, SortField};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::overarching_goal as OverarchingGoalApi;
use domain::{overarching_goals::Model, Id};
use service::config::ApiVersion;

use log::*;

/// POST create a new Overarching Goal
#[utoipa::path(
    post,
    path = "/overarching_goals",
    params(ApiVersion),
    request_body = entity::overarching_goals::Model,
    responses(
        (status = 201, description = "Successfully Created a New Overarching Goal", body = [entity::overarching_goals::Model]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
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
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(overarching_goals_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a New Overarching Goal from: {overarching_goals_model:?}");

    let overarching_goals = OverarchingGoalApi::create(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        overarching_goals_model,
        user.id,
    )
    .await?;

    debug!("New Overarching Goal: {overarching_goals:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        overarching_goals,
    )))
}

/// GET a particular Overarching Goal specified by its id.
#[utoipa::path(
    get,
    path = "/overarching_goals/{id}",
    params(
        ApiVersion,
        ("id" = String, Path, description = "Overarching Goal id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Overarching Goal by its id", body = [entity::notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Overarching Goal not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Overarching Goal by id: {id}");

    let overarching_goal = OverarchingGoalApi::find_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        overarching_goal,
    )))
}

#[utoipa::path(
    put,
    path = "/overarching_goals/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of overarching_goals to update"),
    ),
    request_body = entity::overarching_goals::Model,
    responses(
        (status = 200, description = "Successfully Updated Overarching Goal", body = [entity::overarching_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(overarching_goals_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Overarching Goal with id: {id}");

    let overarching_goals = OverarchingGoalApi::update(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        overarching_goals_model,
    )
    .await?;

    debug!("Updated Overarching Goal: {overarching_goals:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        overarching_goals,
    )))
}

#[utoipa::path(
    put,
    path = "/overarching_goals/{id}/status",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of overarching goal to update"),
        ("value" = Option<String>, Query, description = "Status value to update"),
    ),
    request_body = entity::actions::Model,
    responses(
        (status = 200, description = "Successfully Updated Overarching Goal", body = [entity::overarching_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update_status(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    Query(status): Query<String>,
    Path(id): Path<Id>,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Overarching Goal Status with id: {id}");

    let overarching_goal = OverarchingGoalApi::update_status(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        status.as_str().into(),
    )
    .await?;

    debug!("Updated Overarching Goal: {overarching_goal:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        overarching_goal,
    )))
}

#[utoipa::path(
    get,
    path = "/overarching_goals",
    params(
        ApiVersion,
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching_session_id"),
        ("sort_by" = Option<crate::params::overarching_goal::SortField>, Query, description = "Sort by field. Valid values: 'title', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "title"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Overarching Goals", body = [entity::overarching_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
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
    debug!("GET all Overarching Goals");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(
        &mut params.sort_by,
        &mut params.sort_order,
        SortField::Title,
    );

    let overarching_goals = OverarchingGoalApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Overarching Goals: {overarching_goals:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        overarching_goals,
    )))
}
