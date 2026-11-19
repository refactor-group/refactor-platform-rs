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
use domain::{
    action as ActionApi, coaching_relationship as CoachingRelationshipApi, coaching_relationships,
    Id,
};
use service::config::ApiVersion;

use log::*;

/// Used by the batch handler to gate the HTTP-layer `assignee` scope check: callers who
/// see all actions in any relationship bypass it; everyone else goes through it.
fn caller_sees_all_actions_in_any<'a>(
    relationships: impl IntoIterator<Item = &'a coaching_relationships::Model>,
    user_id: Id,
) -> bool {
    relationships
        .into_iter()
        .any(|relationship| relationship.sees_all_actions(user_id))
}

/// The batch handler's `assignee` scope check, held only to the relationships the scope names
/// someone in. A caller whose every relationship the scope leaves out gets an empty list
/// instead.
fn check_batch_assignee_visibility(
    relationships: &[coaching_relationships::Model],
    caller_id: Id,
    assignee_scope: Option<&ActionApi::AssigneeScope>,
) -> Result<(), Error> {
    let in_scope: Vec<_> = relationships
        .iter()
        .filter(|relationship| {
            assignee_scope.is_none_or(|scope| scope.user_id_in(relationship, caller_id).is_some())
        })
        .collect();
    let all_left_out = in_scope.is_empty() && !relationships.is_empty();
    if all_left_out || caller_sees_all_actions_in_any(in_scope, caller_id) {
        return Ok(());
    }
    let own_visibility = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller_id };
    check_assignee_visibility(caller_id, &own_visibility, assignee_scope)
}

/// 403s a caller limited to their own actions that scopes `assignee` to anyone other than
/// self.
fn check_assignee_visibility(
    caller_id: Id,
    caller_visibility: &ActionApi::CallerVisibility,
    assignee_scope: Option<&ActionApi::AssigneeScope>,
) -> Result<(), Error> {
    let own_id = match caller_visibility {
        ActionApi::CallerVisibility::Unrestricted => return Ok(()),
        ActionApi::CallerVisibility::OwnAndUnassigned { user_id } => *user_id,
    };

    let permitted = match assignee_scope {
        None => true,
        // In a relationship that limits the caller, the coachee is the caller and no one is a
        // colleague.
        Some(ActionApi::AssigneeScope::Coachee | ActionApi::AssigneeScope::Colleague) => true,
        Some(ActionApi::AssigneeScope::User(id)) => *id == own_id,
        Some(ActionApi::AssigneeScope::Coach) => false,
    };

    if permitted {
        Ok(())
    } else {
        warn!(
            "Caller {caller_id}, limited to their own and unassigned actions, attempted to scope assignee outside them: {assignee_scope:?}"
        );
        Err(WebError::Web(WebErrorKind::ForbiddenAssigneeScope))
    }
}

/// GET actions for a coaching relationship.
///
/// Coachees see only actions assigned to themselves or unassigned.
/// `assignee=X` filters strictly to actions whose assignees contain X
/// (excludes unassigned); omit it for the broad view. `assignee=coach`, `coachee` and
/// `colleague` name the participant holding that role, the one who isn't the caller when both
/// do, and are refused when no participant holds it.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}/actions",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "Organization id"),
        ("relationship_id" = Uuid, Path, description = "Coaching relationship id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved actions for the coaching relationship"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Coachee scoped `assignee` to coach or another user"),
        (status = 422, description = "`assignee=coach` or `assignee=coachee` on a colleague relationship, or `assignee=colleague` on a coaching one; `details.reason` is `assignee_scope_not_applicable`"),
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
    // `CoachingRelationshipAccess` has already verified membership, so this
    // `ok_or` should be unreachable. Fail closed to 401 if the invariant is
    // ever violated rather than silently inheriting a permissive default.
    let caller_visibility = ActionApi::CallerVisibility::for_relationship(user.id, &relationship)
        .ok_or(Error::Web(WebErrorKind::Auth))?;
    check_assignee_visibility(user.id, &caller_visibility, assignee_scope.as_ref())?;

    let mut query_params = params.into_query_params();
    query_params.caller_visibility = caller_visibility;

    query_params.assignee_user_id = assignee_scope
        .map(|scope| ActionApi::assignee_user_id(&relationship, &scope, user.id))
        .transpose()?;

    let actions = ActionApi::find_by_coaching_relationship(
        app_state.db_conn_ref(),
        relationship.id,
        query_params,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), actions)))
}

/// GET actions across all participant relationships, grouped by relationship id.
///
/// Visibility narrows per-relationship by the caller's role (full for coach
/// side, self-or-unassigned for coachee side). A role `assignee` leaves out the relationships
/// where no participant holds it: `coach` and `coachee` skip colleague relationships, and
/// `colleague` lists each colleague relationship's other colleague and skips coaching ones.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/actions",
    params(
        ApiVersion,
        ("organization_id" = Uuid, Path, description = "Organization id"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved batch actions, as `actions_by_relationship` keyed by relationship id. With `assignee=coach` or `assignee=coachee`, colleague relationships are left out; with `assignee=colleague`, coaching relationships are left out and each colleague relationship lists the other colleague's actions"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Coachee scoped `assignee` to coach or another user"),
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

    check_batch_assignee_visibility(&relationships, user.id, assignee_scope.as_ref())?;

    let relationship_actions = ActionApi::find_by_user_relationships(
        app_state.db_conn_ref(),
        &relationships,
        query_params,
        assignee_scope,
        user.id,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::json!({ "actions_by_relationship": relationship_actions }),
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

    fn make_relationship(coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
        let now = chrono::Utc::now().fixed_offset();
        coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id,
            coachee_id,
            kind: domain::coaching_relationship_kind::Kind::Coaching,
            slug: "test".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    // ─── caller_sees_all_actions_in_any tests ─────────────────────────────────

    #[test]
    fn caller_sees_all_actions_in_any_empty_relationships_is_false() {
        let user_id = Id::new_v4();
        assert!(!caller_sees_all_actions_in_any(&[], user_id));
    }

    #[test]
    fn caller_sees_all_actions_in_any_only_coachee_is_false() {
        let user_id = Id::new_v4();
        let other = Id::new_v4();
        let rels = vec![
            make_relationship(other, user_id),
            make_relationship(other, user_id),
        ];
        assert!(!caller_sees_all_actions_in_any(&rels, user_id));
    }

    #[test]
    fn caller_sees_all_actions_in_any_coach_in_at_least_one_is_true() {
        let user_id = Id::new_v4();
        let other = Id::new_v4();
        let rels = vec![
            make_relationship(other, user_id), // coachee here
            make_relationship(user_id, other), // coach here
        ];
        assert!(caller_sees_all_actions_in_any(&rels, user_id));
    }

    #[test]
    fn caller_sees_all_actions_in_any_coach_in_all_is_true() {
        let user_id = Id::new_v4();
        let rels = vec![
            make_relationship(user_id, Id::new_v4()),
            make_relationship(user_id, Id::new_v4()),
        ];
        assert!(caller_sees_all_actions_in_any(&rels, user_id));
    }

    #[test]
    fn caller_sees_all_actions_in_any_unrelated_user_is_false() {
        let user_id = Id::new_v4();
        let other_coach = Id::new_v4();
        let other_coachee = Id::new_v4();
        let rels = vec![make_relationship(other_coach, other_coachee)];
        assert!(!caller_sees_all_actions_in_any(&rels, user_id));
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
    fn own_and_unassigned_allowed_when_assignee_omitted() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller };
        assert!(check_assignee_visibility(caller, &vis, None).is_ok());
    }

    #[test]
    fn own_and_unassigned_allowed_with_coachee_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller };
        assert!(
            check_assignee_visibility(caller, &vis, Some(&ActionApi::AssigneeScope::Coachee))
                .is_ok()
        );
    }

    #[test]
    fn own_and_unassigned_allowed_with_own_uuid_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller };
        assert!(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::User(caller))
        )
        .is_ok());
    }

    #[test]
    fn own_and_unassigned_forbidden_with_coach_scope() {
        let caller = Id::new_v4();
        let vis = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller };
        assert_forbidden(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::Coach),
        ));
    }

    #[test]
    fn own_and_unassigned_forbidden_with_other_user_uuid_scope() {
        let caller = Id::new_v4();
        let other = Id::new_v4();
        let vis = ActionApi::CallerVisibility::OwnAndUnassigned { user_id: caller };
        assert_forbidden(check_assignee_visibility(
            caller,
            &vis,
            Some(&ActionApi::AssigneeScope::User(other)),
        ));
    }
}

#[cfg(test)]
#[path = "actions_controller_sqlite_tests.rs"]
mod sqlite_tests;
