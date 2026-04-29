use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::Id;
use log::*;

/// Checks that the `user_id` in the path matches the authenticated user's id.
/// Users can only manage their own tokens.
pub(crate) async fn manage(
    State(_app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(user_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    if authenticated_user.id == user_id {
        next.run(request).await
    } else {
        error!(
            "Forbidden: user_id {} does not match authenticated_user_id {} when managing tokens",
            user_id, authenticated_user.id
        );
        (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
    }
}

/// Variant for routes with both user_id and token_id in the path.
pub(crate) async fn manage_with_token_id(
    State(_app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path((user_id, _token_id)): Path<(Id, Id)>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    if authenticated_user.id == user_id {
        next.run(request).await
    } else {
        error!(
            "Forbidden: user_id {} does not match authenticated_user_id {} when managing tokens",
            user_id, authenticated_user.id
        );
        (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
    }
}
