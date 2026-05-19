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

/// Used by the batch handler to gate the HTTP-layer `assignee` scope check:
/// coach-anywhere callers bypass it; coachee-only callers go through it.
fn caller_is_coach_in_any(relationships: &[coaching_relationships::Model], user_id: Id) -> bool {
    relationships
        .iter()
        .any(|relationship| relationship.coach_id == user_id)
}

/// 403s a coachee caller that scopes `assignee` to anyone other than self.
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

/// GET actions for a coaching relationship. See `BatchCoacheeActions` v5
/// for visibility rules and the `assignee` strict-contains semantic.
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
        (status = 403, description = "Coachee scoped `assignee` to coach or another user"),
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

/// GET actions across all participant relationships, grouped by coachee
/// user id. See `BatchCoacheeActions` v5 for visibility rules and the
/// `assignee` strict-contains semantic.
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

    // Authorize the assignee scope against the caller's role in any of their
    // relationships. If the caller is *only* a coachee across the returned
    // relationships, an `assignee=coach` request is forbidden. If they are a
    // coach in at least one relationship, the assignee scope is permitted.
    if !caller_is_coach_in_any(&relationships, user.id) {
        let coachee_visibility = ActionApi::CallerVisibility::CoacheeSelf { user_id: user.id };
        check_assignee_visibility(user.id, &coachee_visibility, assignee_scope.as_ref())?;
    }

    let coachee_actions = ActionApi::find_by_user_relationships(
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

    fn make_relationship(coach_id: Id, coachee_id: Id) -> coaching_relationships::Model {
        let now = chrono::Utc::now().fixed_offset();
        coaching_relationships::Model {
            id: Id::new_v4(),
            organization_id: Id::new_v4(),
            coach_id,
            coachee_id,
            slug: "test".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    // ─── caller_is_coach_in_any tests ─────────────────────────────────

    #[test]
    fn caller_is_coach_in_any_empty_relationships_is_false() {
        let user_id = Id::new_v4();
        assert!(!caller_is_coach_in_any(&[], user_id));
    }

    #[test]
    fn caller_is_coach_in_any_only_coachee_is_false() {
        let user_id = Id::new_v4();
        let other = Id::new_v4();
        let rels = vec![
            make_relationship(other, user_id),
            make_relationship(other, user_id),
        ];
        assert!(!caller_is_coach_in_any(&rels, user_id));
    }

    #[test]
    fn caller_is_coach_in_any_coach_in_at_least_one_is_true() {
        let user_id = Id::new_v4();
        let other = Id::new_v4();
        let rels = vec![
            make_relationship(other, user_id), // coachee here
            make_relationship(user_id, other), // coach here
        ];
        assert!(caller_is_coach_in_any(&rels, user_id));
    }

    #[test]
    fn caller_is_coach_in_any_coach_in_all_is_true() {
        let user_id = Id::new_v4();
        let rels = vec![
            make_relationship(user_id, Id::new_v4()),
            make_relationship(user_id, Id::new_v4()),
        ];
        assert!(caller_is_coach_in_any(&rels, user_id));
    }

    #[test]
    fn caller_is_coach_in_any_unrelated_user_is_false() {
        let user_id = Id::new_v4();
        let other_coach = Id::new_v4();
        let other_coachee = Id::new_v4();
        let rels = vec![make_relationship(other_coach, other_coachee)];
        assert!(!caller_is_coach_in_any(&rels, user_id));
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
