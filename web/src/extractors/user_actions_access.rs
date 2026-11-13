use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use domain::search::visibility_for;
use domain::{coaching_relationship, Id};
use log::*;

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, parse_path_id_from_parts, RejectionType},
    AppState,
};

/// Admits the `:user_id` user themselves, or a coach of theirs who is still a member of that
/// relationship's organization. On success, yields the target user id.
pub(crate) struct UserActionsAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for UserActionsAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

        let user_id = parse_path_id_from_parts(parts, "user_id").await?;

        if user.id == user_id {
            return Ok(UserActionsAccess(user_id));
        }

        match coaching_relationship::is_coach_of(
            state.db_conn_ref(),
            &visibility_for(&user),
            user_id,
        )
        .await
        {
            Ok(true) => {
                debug!("Coach {} accessing coachee {user_id}'s actions", user.id);
                return Ok(UserActionsAccess(user_id));
            }
            Ok(false) => {}
            // Deny on error.
            Err(e) => error!("Error checking coaching relationship: {e:?}"),
        }

        error!(
            "Forbidden: user {} cannot access actions for user {user_id}",
            user.id
        );
        Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
    }
}
