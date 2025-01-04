use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use serde::Deserialize;

use entity::Id;
use entity_api::coaching_relationship;

#[derive(Debug, Deserialize)]
pub(crate) struct QueryParams {
    coaching_relationship_id: Id,
}

/// Checks that coaching relationship record referenced by `coaching_relationship_id`
/// exists and that the authenticated user is associated with it.
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<QueryParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let coaching_relationship =
        coaching_relationship::find_by_id(app_state.db_conn_ref(), params.coaching_relationship_id)
            .await
            .unwrap_or_default();
    match coaching_relationship {
        Some(coaching_relationship) => {
            if coaching_relationship.coach_id == user.id
                || coaching_relationship.coachee_id == user.id
            {
                // User has access to coaching relationship
                next.run(request).await
            } else {
                // User does not have access to coaching relationship
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        // coaching relationship with given ID not found
        None => (StatusCode::NOT_FOUND, "NOT FOUND").into_response(),
    }
}
