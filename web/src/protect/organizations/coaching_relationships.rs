use crate::error::domain_error_into_response;
use crate::protect::{Predicate, UserIsAdmin};
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};

use domain::{coaching_relationship as CoachingRelationshipApi, Id};

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// and that the authenticated user is an admin
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn create(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let checks: Vec<Predicate> = vec![Predicate::new(UserIsAdmin, vec![organization_id])];

    crate::protect::authorize(&app_state, authenticated_user, request, next, checks).await
}

/// Checks that the authenticated user is a participant (coach or coachee)
/// in the coaching relationship specified by `relationship_id`.
pub(crate) async fn actions(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let relationship_result: Result<_, domain::error::Error> =
        CoachingRelationshipApi::find_by_id(app_state.db_conn_ref(), relationship_id)
            .await
            .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.includes_user(authenticated_user.id) {
                next.run(request).await
            } else {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => domain_error_into_response(e),
    }
}
