use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::{user as UserApi, users, Id};

use log::*;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    check_user_in_organization(
        &app_state,
        authenticated_user,
        organization_id,
        request,
        next,
    )
    .await
}

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn create(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    check_user_in_organization(
        &app_state,
        authenticated_user,
        organization_id,
        request,
        next,
    )
    .await

    // TODO: Check that the authenticated user is a coach
    // It's not immediately clear whether or not this endpoint will be only for coaches in the future until we work out some of the specifics
    //around the user creation workflow. Ex create user -> assign user to coaching relationship later.
    // Leaving this out at the moment. It may be that we decide on separate endpoints for different "flavors" of user creation.
}

async fn check_user_in_organization(
    app_state: &AppState,
    authenticated_user: users::Model,
    organization_id: Id,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match UserApi::find_by_organization(app_state.db_conn_ref(), organization_id).await {
        Ok(users) => {
            if users.iter().any(|user| user.id == authenticated_user.id) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
            }
        }
        Err(_) => {
            error!("Organization not found with ID {:?}", organization_id);
            (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
        }
    }
}
