use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::{coaching_relationship, coaching_session, goal, Id};
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
            if relationship.includes_user(user.id) {
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

/// Checks that the goal referenced by path `id` belongs to a coaching relationship
/// that the authenticated user is a member of (coach or coachee).
pub(crate) async fn by_id(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let goal = match goal::find_by_id(app_state.db_conn_ref(), id).await {
        Ok(goal) => goal,
        Err(e) => {
            let domain_err: domain::error::Error = e.into();
            error!("Error finding goal for authorization: {domain_err:?}");
            return crate::error::domain_error_into_response(domain_err);
        }
    };

    let relationship_result: Result<_, domain::error::Error> =
        coaching_relationship::find_by_id(app_state.db_conn_ref(), goal.coaching_relationship_id)
            .await
            .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.includes_user(user.id) {
                next.run(request).await
            } else {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => {
            error!("Error authorizing goal by_id: {e:?}");
            crate::error::domain_error_into_response(e)
        }
    }
}

/// Checks that the coaching session referenced by path `coaching_session_id`
/// belongs to a coaching relationship that the authenticated user is a member of.
pub(crate) async fn by_coaching_session_id(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(coaching_session_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let session =
        match coaching_session::find_by_id(app_state.db_conn_ref(), coaching_session_id).await {
            Ok(session) => session,
            Err(e) => {
                let domain_err: domain::error::Error = e.into();
                error!("Error finding session for authorization: {domain_err:?}");
                return crate::error::domain_error_into_response(domain_err);
            }
        };

    let relationship_result: Result<_, domain::error::Error> = coaching_relationship::find_by_id(
        app_state.db_conn_ref(),
        session.coaching_relationship_id,
    )
    .await
    .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.includes_user(user.id) {
                next.run(request).await
            } else {
                (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
            }
        }
        Err(e) => {
            error!("Error authorizing goals by_session_id: {e:?}");
            crate::error::domain_error_into_response(e)
        }
    }
}
