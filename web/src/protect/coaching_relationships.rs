use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};
use axum::{
    extract::{Path, Request, State},
    http::StatusCode,
    middleware::Next,
    response::IntoResponse,
};

use entity::Id;
use entity_api::organization;
use std::collections::HashSet;

/// Checks that the organization record referenced by `organization_id`
/// exists and that the authenticated user is associated with i.t
///  Intended to be given to axum::middleware::from_fn_with_state in the router
pub(crate) async fn index(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(organization_id): Path<Id>,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let user_organization_ids = organization::find_by_user(app_state.db_conn_ref(), user.id)
        .await
        .unwrap_or(vec![])
        .into_iter()
        .map(|org| org.id)
        .collect::<HashSet<Id>>();
    if user_organization_ids.contains(&organization_id) {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "UNAUTHORIZED").into_response()
    }
}
