use crate::params::user::IndexParams;
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Query, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::organization::find_with_coaches_coachees;
use log::error;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(params): Query<IndexParams>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match find_with_coaches_coachees(app_state.db_conn_ref(), params.organization_id).await {
        Ok((_organization, coaches, coachees)) => {
            if coaches.iter().any(|coach| coach.id == user.id)
                || coachees.iter().any(|coachee| coachee.id == user.id)
            {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
            }
        }
        Err(_) => {
            error!("Organization not found with {:?}", params);

            (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
        }
    }
}
