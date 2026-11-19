use crate::controller::ApiResponse;
use crate::error::WebErrorKind;
use crate::extractors::coaching_relationship_access::CoachingRelationshipAccess;
use crate::extractors::organization_admin_access::OrganizationAdminAccess;
use crate::extractors::organization_member_access::OrganizationMemberAccess;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::coaching_relationship::goal_progress::IndexParams as GoalProgressIndexParams;
use crate::params::coaching_relationship::{CreateParams, KindParams};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::audit_event::DatabaseRecorder;
use domain::coaching_relationship::{kind_switch, CoachingRelationshipWithUserNames};
use domain::coaching_relationship_kind::Kind;
use domain::{
    action as ActionApi, coaching_relationship as CoachingRelationshipApi,
    goal_progress as GoalProgressApi, Id,
};
use service::config::ApiVersion;

use log::*;

/// CREATE a new CoachingRelationship.
///
/// Restricted to administrators of the organization named in the path. Idempotent:
/// when the pair already has a relationship of the requested kind in the organization,
/// that relationship is returned rather than a duplicate created. Colleagues are an
/// unordered pair, so a colleague relationship is found whichever person is named first.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "Organization id"),
    ),
    request_body = CreateParams,
    responses(
        (status = 200, description = "The relationship of the requested kind (default coaching), newly created or the existing one for this pair: for coaching, this coach and coachee; for colleagues, these two people in either order. The envelope's status_code is 201 in both cases, so it does not distinguish a reuse from a create", body = domain::coaching_relationship::CoachingRelationshipWithUserNames),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization"),
        (status = 404, description = "Organization, or the user named by `coach_id` or `coachee_id`, not found"),
        (status = 405, description = "Method not allowed"),
        (status = 409, description = "`coach_id` and `coachee_id` name the same user, either user is not a member of the organization, the organization is archived, or the two people already have a relationship of the other kind in the organization in either direction (`details.reason` `pair_already_related`, `details.relationship_id` the existing relationship)"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
    )]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Json(params): Json<CreateParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE new Coaching Relationship from: {params:?}");

    let coaching_relationship: CoachingRelationshipWithUserNames =
        CoachingRelationshipApi::create_for_caller(
            app_state.db_conn_ref(),
            &DatabaseRecorder,
            &authenticated_user,
            organization.id,
            CoachingRelationshipApi::Draft {
                coach_id: params.coach_id,
                coachee_id: params.coachee_id,
                kind: params.kind.unwrap_or(Kind::Coaching),
            },
        )
        .await?;

    debug!(
        "Newly created Coaching Relationship: {:?}",
        coaching_relationship
    );

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_relationship,
    )))
}

/// SWITCH a CoachingRelationship's kind in place.
///
/// Restricted to administrators of the organization named in the path. The relationship keeps
/// its id and history; who may do what follows the new kind from then on. Until the body says
/// `confirmed: true`, nothing changes and the response is a 409 whose `details` list what the
/// switch would change, so the administrator can confirm it first.
#[utoipa::path(
    put,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/kind",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "Organization id"),
        ("relationship_id" = Uuid, Path, description = "Coaching relationship id"),
    ),
    request_body = KindParams,
    responses(
        (status = 200, description = "The relationship as switched, or unchanged when it already had the requested kind and coach", body = domain::coaching_relationship::CoachingRelationshipWithUserNames),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Caller does not administer the organization"),
        (status = 404, description = "Organization not found, or no such relationship in it"),
        (status = 405, description = "Method not allowed"),
        (status = 409, description = "Nothing changed. The organization is archived (`error` `organization_archived`, no `details`), or `details.reason` says why: `participant_not_a_member` (`details.user_id` the participant no longer in `details.organization_id`; re-add them first), `pair_already_related` (`details.relationship_id` the pair's other relationship that leaves no room for the switch), or `kind_switch_needs_confirmation` (`details` also holds the fields of `domain.coaching_relationship.kind_switch.Consequences`; re-send with `confirmed: true` to switch)"),
        (status = 422, description = "Nothing changed: `coach_id` missing for `coaching` (`details.reason` `coach_required`), given for `colleague` (`coach_not_applicable`), or not a participant (`coach_not_a_participant`)"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn switch_kind(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    OrganizationAdminAccess {
        organization,
        authenticated_user,
    }: OrganizationAdminAccess,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
    Json(params): Json<KindParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("SWITCH Coaching Relationship {relationship_id} kind: {params:?}");

    let relationship = kind_switch::switch_for_caller(
        app_state.db_conn_ref(),
        &app_state.config,
        kind_switch::Reporters {
            recorder: &DatabaseRecorder,
            event_publisher: &app_state.event_publisher,
        },
        &authenticated_user,
        organization.id,
        relationship_id,
        params.into(),
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationship)))
}

/// GET a particular CoachingRelationship specified by the organization Id and relationship Id.
///
/// Restricted to participants of the relationship who still belong to its organization,
/// matching what the list read returns for the same caller.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "Organization id to retrieve the CoachingRelationship under"),
        ("relationship_id" = Uuid, Path, description = "CoachingRelationship id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain CoachingRelationship by its id", body = domain::coaching_relationship::CoachingRelationshipWithUserNames),
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
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    CoachingRelationshipAccess(coaching_relationship): CoachingRelationshipAccess,
) -> Result<impl IntoResponse, Error> {
    let relationship_id = coaching_relationship.id;
    debug!("GET CoachingRelationship by id: {relationship_id}");

    let relationship: Option<CoachingRelationshipWithUserNames> =
        CoachingRelationshipApi::get_relationship_with_user_names_for_caller(
            app_state.db_conn_ref(),
            &user,
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
        ("organization_id" = Uuid, Path, description = "Organization id to retrieve CoachingRelationships")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all CoachingRelationships", body = [domain::coaching_relationship::CoachingRelationshipWithUserNames]),
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

    let coaching_relationships: Vec<CoachingRelationshipWithUserNames> =
        CoachingRelationshipApi::find_by_organization_for_user_with_user_names(
            app_state.db_conn_ref(),
            &user,
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
        ("organization_id" = Uuid, Path, description = "Organization id"),
        ("relationship_id" = Uuid, Path, description = "Coaching relationship id"),
        ("status" = Option<domain::status::Status>, Query, description = "Filter by goal status (e.g., 'InProgress')"),
        ("sort_by" = Option<crate::params::coaching_relationship::goal_progress::SortField>, Query, description = "Sort by field. Valid values: 'updated_at', 'status_changed_at', 'created_at'.", example = "updated_at"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc', 'desc'.", example = "desc"),
        ("limit" = Option<u32>, Query, description = "Cap on the number of goals returned. Values above 100 are silently clamped. Omit for unbounded results.", example = 3),
        ("assignee" = Option<String>, Query, description = "Scope action counts / next-due to a specific assignee. Values: 'coach', 'coachee', 'colleague' (case-insensitive), or a user UUID. A role names the participant holding it, the one who isn't the caller when both do: 'colleague' is the caller's colleague. A role no participant holds is refused: 'coach' and 'coachee' on a colleague relationship, 'colleague' on a coaching one. Omit for relationship-wide counts."),
        ("coaching_session_id" = Option<Uuid>, Query, description = "Restrict to goals linked to this coaching session via the session↔goal join table."),
    ),
    responses(
        (status = 200, description = "Successfully retrieved goal progress for the coaching relationship"),
        (status = 400, description = "Invalid query parameter value"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Coaching relationship not found"),
        (status = 422, description = "`assignee=coach` or `assignee=coachee` on a colleague relationship, or `assignee=colleague` on a coaching one; `details.reason` is `assignee_scope_not_applicable`"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn goal_progress(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
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
        Some(ActionApi::AssigneeScope::User(id)) if !relationship.includes_user(id) => {
            return Err(Error::Web(WebErrorKind::Input));
        }
        Some(scope) => Some(ActionApi::assignee_user_id(&relationship, &scope, user.id)?),
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

#[cfg(test)]
#[path = "coaching_relationship_controller_sqlite_tests.rs"]
mod sqlite_tests;
