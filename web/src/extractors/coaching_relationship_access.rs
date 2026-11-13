use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRef, FromRequestParts, Path},
    http::{request::Parts, StatusCode},
};
use domain::{coaching_relationship as CoachingRelationshipApi, coaching_relationships, users, Id};

use crate::{
    extractors::{authenticated_user::AuthenticatedUser, lookup_rejection, RejectionType},
    AppState,
};
use log::*;

/// Checks that the authenticated user is a participant (coach or coachee) in the
/// coaching relationship specified by `relationship_id` in the URL path, and that they
/// still belong to that relationship's organization.
///
/// When the route also carries an `organization_id` segment, the relationship must
/// actually live under it, so the segment addresses rather than decorates.
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
                .map_err(|e| lookup_rejection(e.into()))?;

        // The organization segment addresses the relationship, so a mismatch is a
        // request for something that does not exist at that address. 404 rather than
        // 403, to avoid confirming the relationship lives somewhere else. Skipped when
        // the route carries no organization segment; every current one does.
        if let Some(path_organization_id) = path_params.get("organization_id") {
            if path_organization_id.parse::<Id>() != Ok(relationship.organization_id) {
                return Err((StatusCode::NOT_FOUND, "NOT FOUND".to_string()));
            }
        }

        if !relationship.grants_access_to(&authenticated_user) {
            return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
        }

        Ok(CoachingRelationshipAccess(relationship))
    }
}

/// Loads the relationship and admits `user` only if it grants them access.
pub(crate) async fn authorize_relationship(
    state: &AppState,
    user: &users::Model,
    coaching_relationship_id: Id,
) -> Result<coaching_relationships::Model, RejectionType> {
    debug!("Checking coaching relationship access for relationship_id={coaching_relationship_id}");

    let relationship =
        CoachingRelationshipApi::find_by_id(state.db_conn_ref(), coaching_relationship_id)
            .await
            .map_err(|e| lookup_rejection(e.into()))?;

    if !relationship.grants_access_to(user) {
        return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
    }

    Ok(relationship)
}

/// `authorize_relationship`, then admits only the relationship's coach.
pub(crate) async fn authorize_relationship_coach(
    state: &AppState,
    user: &users::Model,
    coaching_relationship_id: Id,
) -> Result<coaching_relationships::Model, RejectionType> {
    let relationship = authorize_relationship(state, user, coaching_relationship_id).await?;

    if relationship.coach_id != user.id {
        return Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()));
    }

    Ok(relationship)
}

#[cfg(test)]
#[cfg(feature = "mock")]
mod tests {
    use std::sync::Arc;

    use axum::{extract::State, response::Response, routing::get, Router};
    use sea_orm::{DbErr, MockDatabase};

    use super::*;
    use crate::extractors::relationship_access_test_support::{self as support, Caller};

    async fn read_relationship(
        CoachingRelationshipAccess(_relationship): CoachingRelationshipAccess,
    ) -> &'static str {
        "extracted_success"
    }

    async fn authorize_path_relationship(
        State(state): State<AppState>,
        AuthenticatedUser(user): AuthenticatedUser,
        Path(relationship_id): Path<Id>,
    ) -> Result<&'static str, RejectionType> {
        authorize_relationship(&state, &user, relationship_id).await?;
        Ok("authorized")
    }

    fn routes() -> Router<AppState> {
        Router::new()
            .route(
                "/organizations/:organization_id/coaching_relationships/:relationship_id",
                get(read_relationship),
            )
            .route(
                "/coaching_relationships/:relationship_id",
                get(authorize_path_relationship),
            )
    }

    /// Signs the coach in over `db`, then sends `GET uri`.
    async fn coach_gets(db: MockDatabase, w: &support::Fixture, uri: &str) -> Response {
        let db = Arc::new(db.into_connection());
        support::send(routes(), &db, &w.coach.email, "GET", uri, None).await
    }

    fn failing_lookup(w: &support::Fixture) -> MockDatabase {
        w.signed_in(Caller::Coach)
            .append_query_errors([DbErr::Custom("connection lost".to_string())])
    }

    #[tokio::test]
    async fn a_failed_relationship_lookup_is_a_server_error() {
        let w = support::fixture();
        let uri = format!(
            "/organizations/{}/coaching_relationships/{}",
            w.organization_id, w.relationship.id
        );

        let response = coach_gets(failing_lookup(&w), &w, &uri).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn an_unknown_relationship_is_not_found() {
        let w = support::fixture();
        let db = w
            .signed_in(Caller::Coach)
            .append_query_results([Vec::<coaching_relationships::Model>::new()]);
        let uri = format!(
            "/organizations/{}/coaching_relationships/{}",
            w.organization_id,
            Id::new_v4()
        );

        let response = coach_gets(db, &w, &uri).await;

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_failed_relationship_authorization_lookup_is_a_server_error() {
        let w = support::fixture();
        let uri = format!("/coaching_relationships/{}", w.relationship.id);

        let response = coach_gets(failing_lookup(&w), &w, &uri).await;

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
