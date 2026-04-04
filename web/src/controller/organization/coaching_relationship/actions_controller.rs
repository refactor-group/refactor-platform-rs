use crate::controller::ApiResponse;
use crate::extractors::coaching_relationship_access::CoachingRelationshipAccess;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_relationship::action::IndexParams;
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::{action as ActionApi, coaching_relationship as CoachingRelationshipApi};
use service::config::ApiVersion;

use log::*;

/// GET actions for a specific coaching relationship.
///
/// The `CoachingRelationshipAccess` extractor verifies that the authenticated
/// user is a participant (coach or coachee) in the relationship.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/actions",
    params(
        ApiVersion,
        ("organization_id" = String, Path, description = "Organization id"),
        ("relationship_id" = String, Path, description = "Coaching relationship id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved actions for the coaching relationship"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    CoachingRelationshipAccess(relationship): CoachingRelationshipAccess,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET actions for coaching relationship: {}", relationship.id);

    let assignee_scope = params.assignee_scope();
    let mut query_params = params.into_query_params();

    // Resolve role-based assignee scope to a concrete user ID
    query_params.assignee_user_id = assignee_scope.map(|scope| match scope {
        ActionApi::AssigneeScope::Coach => relationship.coach_id,
        ActionApi::AssigneeScope::Coachee => relationship.coachee_id,
        ActionApi::AssigneeScope::User(id) => id,
    });

    let actions = ActionApi::find_by_coaching_relationship(
        app_state.db_conn_ref(),
        relationship.id,
        query_params,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}

/// GET actions across all coaching relationships where the authenticated user
/// is the coach, grouped by coachee user ID.
///
/// Supports an optional `assignee` query parameter to filter by who the actions
/// are assigned to:
/// - `?assignee=coach` — actions assigned to the coach in each relationship
/// - `?assignee=coachee` — actions assigned to the coachee in each relationship
/// - `?assignee={user_id}` — actions assigned to a specific user
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/actions",
    params(
        ApiVersion,
        ("organization_id" = String, Path, description = "Organization id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved batch actions"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    OrganizationMemberAccess(organization_id): OrganizationMemberAccess,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET batch actions for coach {} in organization {}",
        user.id, organization_id
    );

    let assignee_scope = params.assignee_scope();
    let query_params = params.into_query_params();

    let relationships = CoachingRelationshipApi::find_by_coach_and_organization(
        app_state.db_conn_ref(),
        user.id,
        organization_id,
    )
    .await?;

    let coachee_actions = ActionApi::find_by_coach_relationships(
        app_state.db_conn_ref(),
        &relationships,
        query_params,
        assignee_scope,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::json!({ "coachee_actions": coachee_actions }),
    )))
}
