use crate::controller::ApiResponse;
use crate::error::{Error as WebError, WebErrorKind};
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
use domain::{action as ActionApi, coaching_relationship as CoachingRelationshipApi, Id};
use service::config::ApiVersion;

use log::*;

/// Returns an error response if the caller is not permitted to scope the
/// returned action set to the requested `assignee`. Coachees may only ask
/// for their own actions (`assignee=coachee` or `assignee=<self.id>`);
/// anything else is a 403.
fn check_assignee_visibility(
    caller_id: Id,
    caller_visibility: &ActionApi::CallerVisibility,
    assignee_scope: Option<&ActionApi::AssigneeScope>,
) -> Result<(), Error> {
    let coachee_self_id = match caller_visibility {
        ActionApi::CallerVisibility::Unrestricted => return Ok(()),
        ActionApi::CallerVisibility::CoacheeSelf { user_id } => *user_id,
    };

    let permitted = match assignee_scope {
        None => true,
        Some(ActionApi::AssigneeScope::Coachee) => true,
        Some(ActionApi::AssigneeScope::User(id)) => *id == coachee_self_id,
        Some(ActionApi::AssigneeScope::Coach) => false,
    };

    if permitted {
        Ok(())
    } else {
        warn!(
            "Coachee caller {caller_id} attempted to scope assignee outside their visibility: {assignee_scope:?}"
        );
        Err(WebError::Web(WebErrorKind::ForbiddenAssigneeScope))
    }
}

/// GET actions for a specific coaching relationship.
///
/// The `CoachingRelationshipAccess` extractor verifies that the authenticated
/// user is a participant (coach or coachee) in the relationship.
///
/// Supports an optional `assignee` query parameter with **strict-contains**
/// semantics — i.e., the action's assignees must contain the resolved user id.
/// Unassigned actions are *excluded* whenever `assignee` is present:
/// - `?assignee=coach` — actions assigned to the coach of this relationship
/// - `?assignee=coachee` — actions assigned to the coachee of this relationship
/// - `?assignee={user_id}` — actions assigned to a specific user
/// - omit the param to get the broad view (no scope filter); the
///   `caller_visibility` predicate alone narrows the result. **For a coachee
///   caller wanting their own broad view (self-assigned ∪ unassigned), omit
///   `assignee` rather than sending `assignee=coachee`** — the strict-contains
///   semantic would otherwise exclude unassigned actions.
///
/// Coachee callers are limited to actions assigned to themselves or
/// unassigned; the coach's assigned actions are never returned to a coachee.
/// Requesting `assignee=coach` (or any user other than self) as a coachee
/// returns 403 with `error: "forbidden_assignee_scope"`.
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
        (status = 403, description = "Forbidden — coachee attempted to scope to coach or another user"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    OrganizationMemberAccess(_organization_id): OrganizationMemberAccess,
    CoachingRelationshipAccess(relationship): CoachingRelationshipAccess,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET actions for coaching relationship {} (caller {})",
        relationship.id, user.id
    );

    let assignee_scope = params.assignee_scope();
    let caller_visibility = ActionApi::CallerVisibility::for_relationship(user.id, &relationship);
    check_assignee_visibility(user.id, &caller_visibility, assignee_scope.as_ref())?;

    let mut query_params = params.into_query_params();
    query_params.caller_visibility = caller_visibility;

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
/// is a participant (coach or coachee), grouped by coachee user ID.
///
/// Supports an optional `assignee` query parameter with **strict-contains**
/// semantics — the action's assignees must contain the resolved user id, so
/// unassigned actions are excluded whenever `assignee` is present. The
/// resolution is per-relationship (the `coach`/`coachee` role strings resolve
/// to that specific relationship's `coach_id`/`coachee_id`):
/// - `?assignee=coach` — actions assigned to the coach in each relationship
/// - `?assignee=coachee` — actions assigned to the coachee in each relationship
/// - `?assignee={user_id}` — actions assigned to a specific user (UUID form;
///   supported for completeness but not invoked by the FE today)
/// - omit the param to get the broad view (no scope filter); the
///   `caller_visibility` predicate alone narrows the result.
///
/// **FE guidance — when to omit `assignee`:** for the coach's "All" tab and
/// for any coachee caller's broad view, omit the param. Sending
/// `assignee=coachee` from a coachee resolves to their own user id and the
/// strict-contains semantic then excludes unassigned actions — by design, but
/// usually not what a coachee broad view wants. For coach role-toggle tabs
/// ("Coach Actions" / "Coachee Actions") that want a strict-scoped slice,
/// include the param as usual.
///
/// Coachee callers are limited to actions assigned to themselves or unassigned
/// in each relationship; the coach's assigned actions are never returned.
/// Coachees may only pass `assignee=coachee` or `assignee=<self.id>` —
/// requesting any other assignee scope returns 403 with
/// `error: "forbidden_assignee_scope"`. For users who are coach in some
/// relationships and coachee in others within the same organization,
/// visibility is determined per-relationship based on the caller's role in
/// each one.
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
        (status = 403, description = "Forbidden — coachee attempted to scope to coach or another user"),
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
        "GET batch actions for user {} in organization {}",
        user.id, organization_id
    );

    let assignee_scope = params.assignee_scope();
    let query_params = params.into_query_params();

    let relationships = CoachingRelationshipApi::find_by_user_and_organization(
        app_state.db_conn_ref(),
        user.id,
        organization_id,
    )
    .await?;

    // Authorize the assignee scope against the caller's role in any of their
    // relationships. If the caller is *only* a coachee across the returned
    // relationships, an `assignee=coach` request is forbidden. If they are a
    // coach in at least one relationship, the assignee scope is permitted.
    let caller_is_coach_anywhere = relationships
        .iter()
        .any(|relationship| relationship.coach_id == user.id);

    if !caller_is_coach_anywhere {
        let coachee_visibility = ActionApi::CallerVisibility::CoacheeSelf { user_id: user.id };
        check_assignee_visibility(user.id, &coachee_visibility, assignee_scope.as_ref())?;
    }

    let coachee_actions = ActionApi::find_by_coach_relationships(
        app_state.db_conn_ref(),
        &relationships,
        query_params,
        assignee_scope,
        user.id,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::json!({ "coachee_actions": coachee_actions }),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_forbidden(result: Result<(), Error>) {
        match result {
            Err(Error::Web(WebErrorKind::ForbiddenAssigneeScope)) => {}
            other => panic!("expected ForbiddenAssigneeScope, got: {other:?}"),
        }
    }

    #[test]
    fn unrestricted_caller_allowed_with_any_assignee_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::Unrestricted;
        assert!(check_assignee_visibility(caller, &vis, None).is_ok());
        assert!(
            check_assignee_visibility(caller, &vis, Some(&ActionApi::AssigneeScope::Coach)).is_ok()
        );
        assert!(
            check_assignee_visibility(caller, &vis, Some(&ActionApi::AssigneeScope::Coachee))
                .is_ok()
        );
        assert!(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::User(Id::new_v4()))
        )
        .is_ok());
    }

    #[test]
    fn coachee_self_allowed_when_assignee_omitted() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::CoacheeSelf { user_id: caller };
        assert!(check_assignee_visibility(caller, &vis, None).is_ok());
    }

    #[test]
    fn coachee_self_allowed_with_coachee_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::CoacheeSelf { user_id: caller };
        assert!(
            check_assignee_visibility(caller, &vis, Some(&ActionApi::AssigneeScope::Coachee))
                .is_ok()
        );
    }

    #[test]
    fn coachee_self_allowed_with_own_uuid_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::CoacheeSelf { user_id: caller };
        assert!(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::User(caller))
        )
        .is_ok());
    }

    #[test]
    fn coachee_self_forbidden_with_coach_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::CoacheeSelf { user_id: caller };
        assert_forbidden(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::Coach),
        ));
    }

    #[test]
    fn coachee_self_forbidden_with_other_user_uuid_scope() {
        let caller = Id::new_v4();
        let other = Id::new_v4();
        let vis = ActionApi::CallerVisibility::CoacheeSelf { user_id: caller };
        assert_forbidden(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::User(other)),
        ));
    }
}
