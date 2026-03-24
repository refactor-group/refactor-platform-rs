use crate::params::coaching_session::goal::BatchIndexParams;
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

/// Authorizes the batch session-goals endpoint.
///
/// If `coaching_relationship_id` is provided, verifies the user belongs to that relationship.
/// If `coaching_session_ids` is provided, looks up the first session's relationship
/// and verifies membership.
pub(crate) async fn batch_by_session(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<BatchIndexParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    // Determine the coaching_relationship_id to authorize against
    let relationship_id = if let Some(rel_id) = params.coaching_relationship_id {
        rel_id
    } else if let Some(first_session_id) = params.coaching_session_ids.first() {
        match coaching_session::find_by_id(app_state.db_conn_ref(), *first_session_id).await {
            Ok(session) => session.coaching_relationship_id,
            Err(e) => {
                let domain_err: domain::error::Error = e.into();
                error!("Error finding session for batch authorization: {domain_err:?}");
                return crate::error::domain_error_into_response(domain_err);
            }
        }
    } else {
        // No filter provided — will be caught by the handler as 400
        return next.run(request).await;
    };

    let relationship_result: Result<_, domain::error::Error> =
        coaching_relationship::find_by_id(app_state.db_conn_ref(), relationship_id)
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
            error!("Error authorizing batch session goals: {e:?}");
            crate::error::domain_error_into_response(e)
        }
    }
}
