//! This module contains middleware functions for protecting routes that expose JWT operations.
use crate::params::jwt::GenerateCollabTokenParams;
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use entity_api::coaching_session;
use log::*;

/// Checks that coaching relationship record associated with the coaching session
/// referenced by `coaching_session_id exists and that the authenticated user is associated with it.
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn generate_collab_token(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<GenerateCollabTokenParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match coaching_session::find_by_id_with_coaching_relationship(
        app_state.db_conn_ref(),
        params.coaching_session_id,
    )
    .await
    {
        Ok((_coaching_session, coaching_relationship)) => {
            if coaching_relationship.coach_id == user.id
                || coaching_relationship.coachee_id == user.id
            {
                next.run(request).await
            } else {
                // User does not have access to coaching relationship
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => {
            error!("Error authorizing collaboration token generation {:?}", e);

            (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL SERVER ERROR").into_response()
        }
    }
}
