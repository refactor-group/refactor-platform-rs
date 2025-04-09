use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use serde::Deserialize;

use domain::{coaching_relationship, coaching_session, Id};
use log::error;
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
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
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
    let coaching_session =
        match coaching_session::find_by_id(app_state.db_conn_ref(), coaching_session_id).await {
            Ok(session) => session,
            Err(e) => {
                error!("Authorization error finding coaching session: {:?}", e);
                return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response();
            }
        };

    let coaching_relationship = match coaching_relationship::find_by_id(
        app_state.db_conn_ref(),
        coaching_session.coaching_relationship_id,
    )
    .await
    {
        Ok(relationship) => relationship,
        Err(e) => {
            error!("Authorization error finding coaching relationship: {:?}", e);
            return (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response();
        }
    };

    if coaching_relationship.coach_id == user.id {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
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
    let coaching_session =
        match coaching_session::find_by_id(app_state.db_conn_ref(), coaching_session_id).await {
            Ok(session) => session,
            Err(e) => {
                error!("Authorization error finding coaching session: {:?}", e);
                return (StatusCode::NOT_FOUND, "NOT FOUND").into_response();
            }
        };

    let coaching_relationship = match coaching_relationship::find_by_id(
        app_state.db_conn_ref(),
        coaching_session.coaching_relationship_id,
    )
    .await
    {
        Ok(relationship) => relationship,
        Err(e) => {
            error!("Authorization error finding coaching relationship: {:?}", e);
            return (StatusCode::NOT_FOUND, "NOT FOUND").into_response();
        }
    };

    if coaching_relationship.coach_id == user.id {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
    }
}
