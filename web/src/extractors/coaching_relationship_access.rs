use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_relationship as CoachingRelationshipApi, coaching_relationships, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is a participant (coach or coachee)
/// in the coaching relationship specified by `relationship_id` in the URL path.
///
/// On success, yields the coaching relationship model so the handler can use it
/// without an additional database query.
pub(crate) struct CoachingRelationshipAccess(pub coaching_relationships::Model);

#[async_trait]
impl<S> FromRequestParts<S> for CoachingRelationshipAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);

        let Path(path_params) = Path::<HashMap<String, String>>::from_request_parts(parts, &state)
            .await
            .map_err(|_| {
                (
                    StatusCode::BAD_REQUEST,
                    "Invalid path parameters".to_string(),
                )
            })?;

        let relationship_id_str = path_params.get("relationship_id").ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "Missing relationship_id in path".to_string(),
            )
        })?;

        let relationship_id = relationship_id_str.parse::<Id>().map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                "Invalid relationship id".to_string(),
            )
        })?;

        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, &state).await?;

        debug!("Checking coaching relationship access for relationship_id={relationship_id}");

        let relationship =
            CoachingRelationshipApi::find_by_id(state.db_conn_ref(), relationship_id)
                .await
                .map_err(|e| {
                    error!("Error finding coaching relationship {relationship_id}: {e:?}");
                    (StatusCode::NOT_FOUND, "NOT FOUND".to_string())
                })?;

        if !relationship.includes_user(authenticated_user.id) {
            return Err((StatusCode::UNAUTHORIZED, "UNAUTHORIZED".to_string()));
        }

        Ok(CoachingRelationshipAccess(relationship))
    }
}
