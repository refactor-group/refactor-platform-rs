use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use crate::params::user::coaching_session::{IncludeParam, IndexParams};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{coaching_session as CoachingSessionApi, Id};
use service::config::ApiVersion;

use log::*;

/// GET all coaching sessions for a specific user with optional related data
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching_sessions",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve coaching sessions for"),
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

    // Build include options from parameters
    let includes = CoachingSessionApi::IncludeOptions {
        relationship: params.include.contains(&IncludeParam::Relationship),
        organization: params.include.contains(&IncludeParam::Organization),
        goal: params.include.contains(&IncludeParam::Goal),
        agreements: params.include.contains(&IncludeParam::Agreements),
    };

    // Convert web layer sort params to entity_api types
    let entity_sort_by = params.sort_by.map(|sf| match sf {
        SortField::Date => CoachingSessionApi::SortField::Date,
        SortField::CreatedAt => CoachingSessionApi::SortField::CreatedAt,
        SortField::UpdatedAt => CoachingSessionApi::SortField::UpdatedAt,
    });

    let entity_sort_order = params.sort_order.map(|so| match so {
        SortOrder::Asc => CoachingSessionApi::SortOrder::Asc,
        SortOrder::Desc => CoachingSessionApi::SortOrder::Desc,
    });

    // Fetch sessions with optional includes and sorting at database level
    let enriched_sessions = CoachingSessionApi::find_by_user_with_includes(
        app_state.db_conn_ref(),
        user_id,
        params.from_date,
        params.to_date,
        entity_sort_by,
        entity_sort_order,
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
