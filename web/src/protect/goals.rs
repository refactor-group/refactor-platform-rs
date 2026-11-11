use crate::params::coaching_session::goal::BatchIndexParams;
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use domain::{coaching_relationship, coaching_session, goal, users, Id};
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
            if relationship.grants_access_to(&user) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
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
            if relationship.grants_access_to(&user) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
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
            if relationship.grants_access_to(&user) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
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
/// If `coaching_session_ids` is provided, verifies the user belongs to every session's
/// relationship.
pub(crate) async fn batch_by_session(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<BatchIndexParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let Some(relationship_id) = params.coaching_relationship_id else {
        // Session ids, or no filter at all, which the handler refuses with a 400.
        return match authorize_sessions(&app_state, &user, &params.coaching_session_ids).await {
            Ok(()) => next.run(request).await,
            Err(rejection) => *rejection,
        };
    };

    let relationship_result: Result<_, domain::error::Error> =
        coaching_relationship::find_by_id(app_state.db_conn_ref(), relationship_id)
            .await
            .map_err(Into::into);

    match relationship_result {
        Ok(relationship) => {
            if relationship.grants_access_to(&user) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
            }
        }
        Err(e) => {
            error!("Error authorizing batch session goals: {e:?}");
            crate::error::domain_error_into_response(e)
        }
    }
}

/// Admits `user` only if every session's relationship grants them access.
async fn authorize_sessions(
    app_state: &AppState,
    user: &users::Model,
    coaching_session_ids: &[Id],
) -> Result<(), Box<Response>> {
    for coaching_session_id in coaching_session_ids {
        let (_, relationship) = coaching_session::find_by_id_with_coaching_relationship(
            app_state.db_conn_ref(),
            *coaching_session_id,
        )
        .await
        .map_err(|e| {
            error!("Error finding session for batch authorization: {e:?}");
            Box::new(crate::error::domain_error_into_response(e))
        })?;

        if !relationship.grants_access_to(user) {
            return Err(Box::new(
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[cfg(feature = "mock")]
#[path = "goals_tests.rs"]
mod tests;
