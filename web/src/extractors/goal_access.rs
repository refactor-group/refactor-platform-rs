use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{goal as GoalApi, goals};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser,
        coaching_relationship_access::authorize_relationship, lookup_rejection,
        parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the relationship that owns the `:id`
/// goal.
///
/// An unknown goal is a 404 and a non-participant a 403, as for [`authorize_relationship`].
/// On success, yields the goal.
pub(crate) struct GoalAccess {
    pub goal: goals::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for GoalAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let goal_id = parse_path_id_from_parts(parts, "id").await?;

        let goal = GoalApi::find_by_id(app_state.db_conn_ref(), goal_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?;

        authorize_relationship(&app_state, &user, goal.coaching_relationship_id).await?;

        Ok(GoalAccess { goal })
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "goal_access_tests.rs"]
mod tests;
