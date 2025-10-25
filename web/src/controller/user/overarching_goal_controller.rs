use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::overarching_goal::{IndexParams, SortField};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{overarching_goal as OverarchingGoalApi, Id};
use serde::Deserialize;
use service::config::ApiVersion;

use log::*;

#[derive(Debug, Deserialize)]
pub(crate) struct QueryParams {
    pub(crate) coaching_session_id: Option<Id>,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<crate::params::sort::SortOrder>,
}

/// GET all overarching goals for a specific user
#[utoipa::path(
    get,
    path = "/users/{user_id}/overarching_goals",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve overarching goals for"),
        ("coaching_session_id" = Option<Id>, Query, description = "Filter by coaching_session_id"),
        ("sort_by" = Option<crate::params::user::overarching_goal::SortField>, Query, description = "Sort by field. Valid values: 'title', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "title"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved overarching goals for user", body = [domain::overarching_goals::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "User not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Query(query_params): Query<QueryParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Overarching Goals for User: {user_id}");
    debug!("Filter Params: {query_params:?}");

    // Build params with user_id from path
    let mut params = IndexParams::new(user_id).with_filters(
        query_params.coaching_session_id,
        query_params.sort_by,
        query_params.sort_order,
    );

    // Apply default sorting parameters
    IndexParams::apply_sort_defaults(
        &mut params.sort_by,
        &mut params.sort_order,
        SortField::Title,
    );

    let overarching_goals = OverarchingGoalApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found {} overarching goals for user {user_id}", overarching_goals.len());

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), overarching_goals)))
}
