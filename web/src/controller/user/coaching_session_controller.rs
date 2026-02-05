use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::coaching_session::{IncludeParam, IndexParams};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_session as CoachingSessionApi, Id, QuerySort};
use service::config::ApiVersion;

use log::*;

/// GET all coaching sessions for a specific user with optional related data
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching_sessions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve coaching sessions for"),
        ("coaching_relationship_id" = Option<Id>, Query, description = "Filter sessions to only those in this coaching relationship"),
        ("from_date" = Option<chrono::NaiveDate>, Query, description = "Filter by from_date (inclusive, UTC)"),
        ("to_date" = Option<chrono::NaiveDate>, Query, description = "Filter by to_date (inclusive, UTC)"),
        ("include" = Option<String>, Query, description = "Comma-separated list of related resources to include. Valid values: 'relationship', 'organization', 'goal', 'agreements'. Example: 'relationship,organization,goal'"),
        ("sort_by" = Option<crate::params::coaching_session::SortField>, Query, description = "Sort by field. Valid values: 'date', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "date"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved coaching sessions for user", body = [domain::coaching_session::EnrichedSession]),
        (status = 400, description = "Bad Request - Invalid include parameter"),
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
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Coaching Sessions for User: {user_id}");
    debug!("Query Params: {params:?}");

    // Set user_id from path parameter and apply defaults
    let params = params.with_user_id(user_id).apply_defaults();

    // Build include options from parameters
    let includes = CoachingSessionApi::IncludeOptions {
        relationship: params.include.contains(&IncludeParam::Relationship),
        organization: params.include.contains(&IncludeParam::Organization),
        goal: params.include.contains(&IncludeParam::Goal),
        agreements: params.include.contains(&IncludeParam::Agreements),
    };
    let sort_column = params.get_sort_column();
    let sort_order = params.get_sort_order();

    // Fetch sessions with optional includes and sorting at database level
    let enriched_sessions = CoachingSessionApi::find_by_user_with_includes(
        app_state.db_conn_ref(),
        user_id,
        params.coaching_relationship_id,
        params.from_date,
        params.to_date,
        sort_column,
        sort_order,
        includes,
    )
    .await?;

    debug!(
        "Found {} coaching sessions for user {user_id}",
        enriched_sessions.len()
    );

    // Return entity_api type directly - it's already serializable
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        enriched_sessions,
    )))
}
