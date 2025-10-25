use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use crate::params::user::coaching_session::IndexParams;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveDate;
use domain::{coaching_sessions::Model, Id};
use entity_api::coaching_session as CoachingSessionApi;
use serde::Deserialize;
use service::config::ApiVersion;

use log::*;

#[derive(Debug, Deserialize)]
pub(crate) struct QueryParams {
    pub(crate) from_date: Option<NaiveDate>,
    pub(crate) to_date: Option<NaiveDate>,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
}

/// GET all coaching sessions for a specific user
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching_sessions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve coaching sessions for"),
        ("from_date" = Option<NaiveDate>, Query, description = "Filter by from_date"),
        ("to_date" = Option<NaiveDate>, Query, description = "Filter by to_date"),
        ("sort_by" = Option<crate::params::coaching_session::SortField>, Query, description = "Sort by field. Valid values: 'date', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "date"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved coaching sessions for user", body = [domain::coaching_sessions::Model]),
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
    debug!("GET Coaching Sessions for User: {user_id}");
    debug!("Filter Params: {query_params:?}");

    // Fetch all coaching sessions for the user (where they are coach or coachee)
    let mut sessions = CoachingSessionApi::find_by_user(app_state.db_conn_ref(), user_id).await?;

    // Apply date range filters
    if let Some(from_date) = query_params.from_date {
        sessions.retain(|session| session.date.date() >= from_date);
    }
    if let Some(to_date) = query_params.to_date {
        sessions.retain(|session| session.date.date() <= to_date);
    }

    // Apply sorting
    if let (Some(sort_by), Some(sort_order)) = (query_params.sort_by, query_params.sort_order) {
        let ascending = matches!(sort_order, SortOrder::Asc);
        sessions.sort_by(|a, b| {
            let cmp = match sort_by {
                SortField::Date => a.date.cmp(&b.date),
                SortField::CreatedAt => a.created_at.cmp(&b.created_at),
                SortField::UpdatedAt => a.updated_at.cmp(&b.updated_at),
            };
            if ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });
    }

    debug!("Found {} coaching sessions for user {user_id}", sessions.len());

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), sessions)))
}
