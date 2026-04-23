use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::coaching_relationship_access::CoachingRelationshipAccess;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_relationship::goal_progress::IndexParams as GoalProgressIndexParams;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_relationship::CoachingRelationshipWithUserNames;
use domain::{
    action as ActionApi, coaching_relationship as CoachingRelationshipApi, coaching_relationships,
    goal_progress as GoalProgressApi, Id,
};
use service::config::ApiVersion;

use log::*;

/// CREATE a new CoachingRelationship.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
    ),
    request_body = entity::coaching_relationships::Model,
    responses(
        (status = 200, description = "Successfully created a new Coaching Relationship", body = [coaching_relationships::Model]),
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
    State(app_state): State<AppState>,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
    Json(coaching_relationship_model): Json<coaching_relationships::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE new Coaching Relationship from: {coaching_relationship_model:?}");

    let coaching_relationship: CoachingRelationshipWithUserNames = CoachingRelationshipApi::create(
        app_state.db_conn_ref(),
        organization_id,
        coaching_relationship_model,
    )
    .await?;

    debug!(
        "Newly created Coaching Relationship: {:?}",
        &coaching_relationship
    );

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_relationship,
    )))
}

/// GET a particular CoachingRelationship specified by the organization Id and relationship Id.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve the CoachingRelationship under"),
        ("relationship_id" = String, Path, description = "CoachingRelationship id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain CoachingRelationship by its id", body = [coaching_relationships::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "CoachingRelationship not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET CoachingRelationship by id: {relationship_id}");

    let relationship: Option<CoachingRelationshipWithUserNames> =
        CoachingRelationshipApi::get_relationship_with_user_names(
            app_state.db_conn_ref(),
            relationship_id,
        )
        .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationship)))
}

/// GET all CoachingRelationships by organization_id
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve CoachingRelationships")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all CoachingRelationships", body = [coaching_relationships::Model]),
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
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET all CoachingRelationships for user {} in organization {}",
        user.id, organization_id
    );

    let coaching_relationships =
        CoachingRelationshipApi::find_by_organization_for_user_with_user_names(
            app_state.db_conn_ref(),
            user.id,
            organization_id,
        )
        .await?;

    debug!("Found CoachingRelationships: {coaching_relationships:?}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_relationships,
    )))
}

/// GET aggregate goal progress for all goals in a coaching relationship.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/goal_progress",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id"),
        ("relationship_id" = Id, Path, description = "Coaching relationship id"),
        ("status" = Option<domain::status::Status>, Query, description = "Filter by goal status (e.g., 'InProgress')"),
        ("sort_by" = Option<crate::params::coaching_relationship::goal_progress::SortField>, Query, description = "Sort by field. Valid values: 'updated_at', 'status_changed_at', 'created_at'.", example = "updated_at"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc', 'desc'.", example = "desc"),
        ("limit" = Option<u32>, Query, description = "Cap on the number of goals returned. Values above 100 are silently clamped. Omit for unbounded results.", example = 3),
        ("assignee" = Option<String>, Query, description = "Scope action counts / next-due to a specific assignee. Values: 'coach', 'coachee' (case-insensitive), or a user UUID. Omit for relationship-wide counts."),
        ("coaching_session_id" = Option<Id>, Query, description = "Restrict to goals linked to this coaching session via the session↔goal join table."),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goal progress for the coaching relationship"),
        (status = 400, description = "Invalid query parameter value"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching relationship not found"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn goal_progress(
    CompareApiVersion(_v): CompareApiVersion,
    CoachingRelationshipAccess(relationship): CoachingRelationshipAccess,
    State(app_state): State<AppState>,
    Query(params): Query<GoalProgressIndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET goal progress for coaching relationship: {}",
        relationship.id
    );
    debug!("Filter Params: {params:?}");

    let assignee_scope = params.assignee_scope();
    let mut query_params = params.into_query_params();

    // Resolve role-based assignee scope against the relationship model.
    // A UUID-valued scope must match the coach or coachee of this relationship —
    // otherwise reject with 400 rather than silently returning zero-filled stats
    // (which would expose a probe oracle for arbitrary user ids).
    query_params.assignee_user_id = match assignee_scope {
        Some(ActionApi::AssigneeScope::Coach) => Some(relationship.coach_id),
        Some(ActionApi::AssigneeScope::Coachee) => Some(relationship.coachee_id),
        Some(ActionApi::AssigneeScope::User(id))
            if id == relationship.coach_id || id == relationship.coachee_id =>
        {
            Some(id)
        }
        Some(ActionApi::AssigneeScope::User(_)) => {
            return Err(Error::Web(WebErrorKind::Input));
        }
        None => None,
    };

    let progress = GoalProgressApi::relationship_goal_progress(
        app_state.db_conn_ref(),
        relationship.id,
        query_params,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), progress)))
}
