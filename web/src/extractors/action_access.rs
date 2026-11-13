use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::request::Parts,
};
use domain::{action as ActionApi, actions, coaching_relationships};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, coaching_session_access::authorize_session,
        lookup_rejection, parse_path_id_from_parts, RejectionType,
    },
    AppState,
};

/// Verifies the authenticated user is a participant of the relationship that owns the `:id`
/// action, by way of the action's coaching session.
///
/// An unknown action is a 404; an unknown session is a 404 and a non-participant a 403, as
/// for [`authorize_session`]. On success, yields the action and its relationship.
pub(crate) struct ActionAccess {
    pub action: actions::Model,
    pub relationship: coaching_relationships::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for ActionAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app_state = AppState::from_ref(state);

        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let action_id = parse_path_id_from_parts(parts, "id").await?;

        let action = ActionApi::find_by_id(app_state.db_conn_ref(), action_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?;

        let (_, relationship) =
            authorize_session(&app_state, &user, action.coaching_session_id).await?;

        Ok(ActionAccess {
            action,
            relationship,
        })
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "action_access_tests.rs"]
mod tests;
