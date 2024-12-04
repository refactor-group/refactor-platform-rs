use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};

use entity::Id;
use entity_api::coaching_relationship;
use std::collections::HashMap;

/// Checks that coaching relationship record referenced by `coaching_relationship_id`
/// exists and that the authenticated user is associated with it.
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<HashMap<String, String>>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    if let Some(coaching_relationship_id) = params.get("coaching_relationship_id") {
        let coaching_relationship_id = match Id::try_parse(coaching_relationship_id) {
            Ok(id) => id,
            Err(_) => {
                // coaching relationship ID is not a parseable UUID
                return (StatusCode::BAD_REQUEST, "BAD REQUEST").into_response();
            }
        };
        let coaching_relationship =
            coaching_relationship::find_by_id(app_state.db_conn_ref(), coaching_relationship_id)
                .await
                .unwrap_or(None);
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
    } else {
        // No coaching relationship ID provided
        (StatusCode::BAD_REQUEST, "BAD REQUEST").into_response()
    }
}
