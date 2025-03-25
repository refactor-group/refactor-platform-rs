use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};
use domain::{user, Id};
use log::error;

/// Checks that the authenticated user is associated with the organization specified by `organization_id`
/// Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(authenticated_user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    match user::find_by_organization(app_state.db_conn_ref(), organization_id).await {
        Ok(users) => {
            if users.iter().any(|user| user.id == authenticated_user.id) {
                next.run(request).await
            } else {
                (StatusCode::FORBIDDEN, "FORBIDDEN").into_response()
            }
        }
        Err(_) => {
            error!("Organization not found with ID {:?}", organization_id);

            (StatusCode::NOT_FOUND, "NOT FOUND").into_response()
        }
    }
}
