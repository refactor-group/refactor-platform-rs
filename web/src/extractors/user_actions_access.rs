use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts},
    http::{request::Parts, StatusCode},
};
use domain::search::visibility_for;
use domain::{coaching_relationship, Id};
use log::*;

use crate::{
    extractors::{
        authenticated_user::AuthenticatedUser, lookup_rejection, parse_path_id_from_parts,
        RejectionType,
    },
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

        let is_coach = coaching_relationship::is_coach_of(
            state.db_conn_ref(),
            &visibility_for(&user),
            user_id,
        )
        .await
        .map_err(|e| {
            error!("Error checking coaching relationship: {e:?}");
            lookup_rejection(e.into())
        })?;

        if is_coach {
            debug!("Coach {} accessing coachee {user_id}'s actions", user.id);
            return Ok(UserActionsAccess(user_id));
        }

        error!(
            "Forbidden: user {} cannot access actions for user {user_id}",
            user.id
        );
        Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
    }
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::{routing::get, Router};
    use sea_orm::DbErr;

    use super::*;
    use crate::extractors::relationship_access_test_support::{self as support, Caller};

    async fn protected_route(_access: UserActionsAccess) -> &'static str {
        "extracted_success"
    }

    #[tokio::test]
    async fn a_failed_coach_lookup_is_a_server_error() {
        let w = support::fixture();
        let db = Arc::new(
            w.signed_in(Caller::Coach)
                .append_query_errors([DbErr::Custom("connection lost".to_string())])
                .into_connection(),
        );
        let routes = Router::new().route("/users/:user_id/actions", get(protected_route));

        let response = support::send(
            routes,
            &db,
            &w.coach.email,
            "GET",
            &format!("/users/{}/actions", w.coachee.id),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
