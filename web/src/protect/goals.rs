use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::{coaching_relationship, Id};
use log::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct QueryParams {
    coaching_relationship_id: Id,
}

/// Checks that the coaching relationship referenced by `coaching_relationship_id` exists
/// and that the authenticated user is either the coach or coachee in it.
/// Intended to be given to axum::middleware::from_fn_with_state in the router.
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<QueryParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let relationship_result: Result<_, domain::error::Error> =
        coaching_relationship::find_by_id(app_state.db_conn_ref(), params.coaching_relationship_id)
            .await
            .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.coach_id == user.id || relationship.coachee_id == user.id {
                next.run(request).await
            } else {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => {
            error!("Error authorizing goals index: {e:?}");
            crate::error::domain_error_into_response(e)
        }
    }
}
