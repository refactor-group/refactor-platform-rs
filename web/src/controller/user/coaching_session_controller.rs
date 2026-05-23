use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::user::coaching_session::{
    CountsByMonthParams, GroupByParam, IncludeParam, IndexParams,
};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono_tz::Tz;
use domain::{coaching_session as CoachingSessionApi, Id, QuerySort};
use serde::Serialize;
use service::config::ApiVersion;
use std::str::FromStr;
use utoipa::ToSchema;

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
        CoachingSessionApi::SessionQueryOptions {
            coaching_relationship_id: params.coaching_relationship_id,
            from_date: params.from_date,
            to_date: params.to_date,
            sort_column,
            sort_order,
            includes,
        },
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

/// Payload returned under `ApiResponse::data` for the counts endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CountsResponse {
    pub counts: Vec<CoachingSessionApi::CountByMonth>,
}

/// GET monthly coaching-session counts for a specific user.
///
/// Aggregates by local calendar month in the caller-supplied IANA timezone
/// (`?tz=`). Months with zero sessions are omitted; results are sorted
/// ascending chronologically. Authentication is required; the protect
/// middleware further restricts the caller to their own `user_id`.
#[utoipa::path(
    get,
    path = "/users/{user_id}/coaching_sessions/counts",
    params(
        ApiVersion,
        ("user_id" = Id, Path, description = "User ID to retrieve counts for"),
        ("from_date" = chrono::NaiveDate, Query, description = "Start of the range (inclusive)"),
        ("to_date" = chrono::NaiveDate, Query, description = "End of the range (inclusive at calendar-day precision)"),
        ("group_by" = crate::params::user::coaching_session::GroupByParam, Query, description = "Aggregation grouping. v1 accepts only 'month'."),
        ("tz" = String, Query, description = "IANA timezone identifier (e.g. 'America/Los_Angeles'). Invalid value → 400 invalid_timezone."),
        ("coaching_relationship_id" = Option<Id>, Query, description = "Narrow to a single coaching relationship.")
    ),
    responses(
        (status = 200, description = "Monthly counts in the requested timezone", body = CountsResponse),
        (status = 400, description = "Bad request (e.g. invalid timezone, malformed query)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden (cross-user request)"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(("cookie_auth" = []))
)]
pub async fn counts(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Path(user_id): Path<Id>,
    Query(params): Query<CountsByMonthParams>,
) -> Result<impl IntoResponse, Error> {
    let params = params.with_user_id(user_id);
    debug!(
        "GET coaching session counts for user {user_id}, params: from={} to={} tz={} relationship={:?}",
        params.from_date, params.to_date, params.tz, params.coaching_relationship_id
    );

    // The only v1 grouping; serde already rejects anything else with 400.
    // The match forces a compile-error if `GroupByParam` ever grows so we
    // don't silently swallow a new variant.
    match params.group_by {
        GroupByParam::Month => (),
    }

    // Validate the IANA timezone identifier before touching the DB. On failure
    // surface a structured 400 with `error: "invalid_timezone"` so callers can
    // branch on the discriminator (see web/src/error.rs).
    let tz = Tz::from_str(&params.tz)
        .map_err(|_| Error::Web(WebErrorKind::InvalidTimezone(params.tz.clone())))?;

    let counts = CoachingSessionApi::find_counts_by_month_for_user(
        app_state.db_conn_ref(),
        user_id,
        params.from_date,
        params.to_date,
        tz.name(),
        params.coaching_relationship_id,
    )
    .await?;

    debug!("Returning {} monthly count buckets", counts.len());
    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        CountsResponse { counts },
    )))
}
