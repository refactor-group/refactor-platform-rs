use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::Id;
use log::*;

/// Checks that the `user_id` matches the `authenticated_user.id`
pub(crate) async fn index(
    State(_app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(user_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // check that we are only allowing authenticated users to read their own coachee relationships
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
