use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Query},
    http::request::Parts,
};
use domain::{users, Id};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser,
        coaching_relationship_access::authorize_relationship,
        coaching_session_access::authorize_session, RejectionType,
    },
    params::coaching_session::goal::BatchIndexParams,
    AppState,
};

/// Authorizes the batch session-goals read.
///
/// With `coaching_relationship_id`, the caller must be in that relationship. Otherwise every
/// id in `coaching_session_ids` must be a session of theirs; the first refusal wins. An empty
/// filter passes here so the handler answers it with a 400. Yields nothing.
pub(crate) struct CoachingSessionGoalBatchAccess;

#[async_trait]
impl<S> FromRequestParts<S> for CoachingSessionGoalBatchAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, &state).await?;

        let Query(params) = Query::<BatchIndexParams>::from_request_parts(parts, &state)
            .await
            .map_err(|rejection| (rejection.status(), rejection.body_text()))?;

        match params.coaching_relationship_id {
            Some(relationship_id) => {
                authorize_relationship(&state, &user, relationship_id).await?;
            }
            None => authorize_sessions(&state, &user, &params.coaching_session_ids).await?,
        }

        Ok(CoachingSessionGoalBatchAccess)
    }
}

/// Admits `user` only if every session's relationship grants them access.
async fn authorize_sessions(
    state: &AppState,
    user: &users::Model,
    coaching_session_ids: &[Id],
) -> Result<(), RejectionType> {
    for coaching_session_id in coaching_session_ids {
        authorize_session(state, user, *coaching_session_id).await?;
    }
    Ok(())
}
