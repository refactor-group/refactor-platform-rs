use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::Id;
use log::*;

pub(crate) mod actions;
pub(crate) mod coach_relationships;
pub(crate) mod coachee_relationships;
pub(crate) mod coaching_sessions;
pub(crate) mod organizations;
pub(crate) mod overarching_goals;
pub(crate) mod passwords;
pub(crate) mod relationship_roles_summary;

/// Checks that the `user_id` matches the `authenticated_user.id`
pub(crate) async fn read(
    State(_app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(user_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // check that we are only allowing authenticated users to read themselves (for now)
    if authenticated_user.id == user_id {
        next.run(request).await
    } else {
        error!(
            "Unauthorized: user_id {} does not match authenticated_user_id {}",
            user_id, authenticated_user.id
        );
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

/// Checks that the `user_id` matches the `authenticated_user.id`
pub(crate) async fn update(
    State(_app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(user_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    info!("Authenticated user id: {}", authenticated_user.id);
    // check that we are only allowing authenticated users to update themselves (for now)
    if authenticated_user.id == user_id {
        next.run(request).await
    } else {
        error!(
            "Unauthorized: user_id {} does not match authenticated_user_id {}",
            user_id, authenticated_user.id
        );
        (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}
