use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Query},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_session as CoachingSessionApi, users, Id};

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser,
        coaching_relationship_access::authorize_relationship, lookup_rejection, not_found,
        RejectionType,
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

/// Admits `user` only if every session's relationship grants them access. Loads the sessions
/// in one query, then walks the ids in request order: the first unknown id is a 404, the
/// first session of someone else's relationship a 403.
async fn authorize_sessions(
    state: &AppState,
    user: &users::Model,
    coaching_session_ids: &[Id],
) -> Result<(), RejectionType> {
    let relationships: HashMap<Id, _> = CoachingSessionApi::find_by_ids_with_coaching_relationship(
        state.db_conn_ref(),
        coaching_session_ids,
    )
    .await
    .map_err(|e| lookup_rejection(e.into()))?
    .into_iter()
    .map(|(session, relationship)| (session.id, relationship))
    .collect();

    coaching_session_ids.iter().try_for_each(|id| {
        relationships
            .get(id)
            .ok_or_else(not_found)?
            .grants_access_to(user)
            .then_some(())
            .ok_or_else(|| (StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
    })
}

#[cfg(test)]
#[path = "coaching_session_goal_access_sqlite_tests.rs"]
mod sqlite_tests;
