use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::{coaching_relationship, Id};
use log::*;

/// Checks that the authenticated user can access the target user's actions.
///
/// Access is granted if:
/// - The authenticated user is requesting their own actions (self-access)
/// - The authenticated user is a coach of the target user
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(user_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // Allow self-access
    if authenticated_user.id == user_id {
        return next.run(request).await;
    }

    // Allow coach to access coachee's actions
    match coaching_relationship::is_coach_of(
        app_state.db_conn_ref(),
        authenticated_user.id,
        user_id,
    )
    .await
    {
        Ok(true) => {
            debug!(
                "Coach {} accessing coachee {}'s actions",
                authenticated_user.id, user_id
            );
            return next.run(request).await;
        }
        Ok(false) => {
            // Not a coach of this user, fall through to unauthorized
        }
        Err(e) => {
            error!("Error checking coaching relationship: {e:?}");
            // On error, deny access for safety
        }
    }

    error!(
        "Unauthorized: user {} cannot access actions for user {}",
        authenticated_user.id, user_id
    );
    (StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
}
