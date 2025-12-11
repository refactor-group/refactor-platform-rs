use crate::protect::{Predicate, UserInOrganization, UserIsAdmin};
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    middleware::Next,
    response::IntoResponse,
};

use domain::Id;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let checks: Vec<Predicate> = vec![Predicate::new(UserInOrganization, vec![organization_id])];

    crate::protect::authorize(&app_state, authenticated_user, request, next, checks).await
}

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
    let checks: Vec<Predicate> = vec![
        Predicate::new(UserInOrganization, vec![organization_id]),
        Predicate::new(UserIsAdmin, vec![organization_id]),
    ];

    crate::protect::authorize(&app_state, authenticated_user, request, next, checks).await
}
