use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use serde::Deserialize;

use domain::{coaching_relationship, coaching_session, Id};

use log::{debug, error, warn};
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
            .await;
    match coaching_relationship {
        Ok(coaching_relationship) => {
            if coaching_relationship.coach_id == user.id
                || coaching_relationship.coachee_id == user.id
            {
                // User has access to coaching relationship
                next.run(request).await
            } else {
                // User does not have access to coaching relationship
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
            }
        }
        // coaching relationship with given ID not found
        Err(_) => (StatusCode::NOT_FOUND, "NOT FOUND").into_response(),
    }
}

/// Checks that coaching session record referenced by `coaching_session_id`
///     * exists
///     * that the authenticated user is associated with it.
///     * that the authenticated user is the coach
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn update(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(coaching_session_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let (_coaching_session, coaching_relationship) =
        match coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        {
            Ok(pair) => pair,
            Err(e) => {
                error!(
                    "Error resolving coaching session {coaching_session_id} for authorization: {e:?}"
                );
                return crate::error::domain_error_into_response(e);
            }
        };

    if coaching_relationship.coach_id == user.id {
        debug!(
            "PUT auth passed: coaching_session_id={coaching_session_id} relationship_id={} user_id={}",
            coaching_relationship.id, user.id
        );
        next.run(request).await
    } else {
        warn!(
            "PUT auth denied (not coach): coaching_session_id={coaching_session_id} relationship_id={} user_id={}",
            coaching_relationship.id, user.id
        );
        (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
    }
}

/// Checks that coaching session record referenced by `coaching_session_id`
///     * exists
///     * that the authenticated user is associated with it.
///     * that the authenticated user is the coach
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn delete(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(coaching_session_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let (_coaching_session, coaching_relationship) =
        match coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        {
            Ok(pair) => pair,
            Err(e) => {
                error!(
                    "Error resolving coaching session {coaching_session_id} for authorization: {e:?}"
                );
                return crate::error::domain_error_into_response(e);
            }
        };

    if coaching_relationship.coach_id == user.id {
        debug!(
            "DELETE auth passed: coaching_session_id={coaching_session_id} relationship_id={} user_id={}",
            coaching_relationship.id, user.id
        );
        next.run(request).await
    } else {
        warn!(
            "DELETE auth denied (not coach): coaching_session_id={coaching_session_id} relationship_id={} user_id={}",
            coaching_relationship.id, user.id
        );
        (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
    }
}
