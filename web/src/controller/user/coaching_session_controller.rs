use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use crate::params::user::coaching_session::{IncludeParam, IndexParams};
use crate::response::coaching_session::EnrichedCoachingSession;
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
        (status = 200, description = "Successfully retrieved coaching sessions for user", body = [crate::response::coaching_session::EnrichedCoachingSession]),
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

    // Validate include parameters
    if let Err(err_msg) = params.validate_includes() {
        warn!("Invalid include parameters: {err_msg}");
        return Err(Error::Web(WebErrorKind::Input));
    }

    // Build include options from parameters
    let includes = CoachingSessionApi::IncludeOptions {
        relationship: params.include.contains(&IncludeParam::Relationship),
        organization: params.include.contains(&IncludeParam::Organization),
        goal: params.include.contains(&IncludeParam::Goal),
        agreements: params.include.contains(&IncludeParam::Agreements),
    };

    // Fetch sessions with optional includes
    let mut enriched_sessions = CoachingSessionApi::find_by_user_with_includes(
        app_state.db_conn_ref(),
        user_id,
        params.from_date,
        params.to_date,
        includes,
    )
    .await?;

    // Apply sorting
    if let (Some(sort_by), Some(sort_order)) = (params.sort_by, params.sort_order) {
        let ascending = matches!(sort_order, SortOrder::Asc);
        enriched_sessions.sort_by(|a, b| {
            let cmp = match sort_by {
                SortField::Date => a.session.date.cmp(&b.session.date),
                SortField::CreatedAt => a.session.created_at.cmp(&b.session.created_at),
                SortField::UpdatedAt => a.session.updated_at.cmp(&b.session.updated_at),
            };
            if ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });
    }

    debug!(
        "Found {} coaching sessions for user {user_id}",
        enriched_sessions.len()
    );

    // Convert to response DTOs
    let response_sessions: Vec<EnrichedCoachingSession> = enriched_sessions
        .into_iter()
        .map(|enriched| convert_to_response(enriched))
        .collect();

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        response_sessions,
    )))
}

/// Convert entity_api EnrichedSession to web response DTO
fn convert_to_response(enriched: CoachingSessionApi::EnrichedSession) -> EnrichedCoachingSession {
    let mut response = EnrichedCoachingSession::from_model(enriched.session);

    // Add relationship data if present
    if let (Some(relationship), Some(coach), Some(coachee)) =
        (enriched.relationship, enriched.coach, enriched.coachee)
    {
        response = response.with_relationship(relationship, coach, coachee);
    }

    // Add organization data if present
    if let Some(organization) = enriched.organization {
        response = response.with_organization(organization);
    }

    // Add goal data if present
    if let Some(goal) = enriched.overarching_goal {
        response = response.with_goal(goal);
    }

    // Add agreement data if present
    if let Some(agreement) = enriched.agreement {
        response = response.with_agreement(agreement);
    }

    response
}
