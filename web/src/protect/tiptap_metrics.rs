//! SuperAdmin gate for /admin/tiptap/metrics/* endpoints.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::IntoResponse,
};

use crate::protect::{authorize, Predicate, UserIsAdmin};
use crate::{extractors::authenticated_user::AuthenticatedUser, AppState};

/// Admin-only gate. `UserIsAdmin` with empty args falls through to a
/// SuperAdmin-only check (see protect/mod.rs:194 comment). This is the
/// codebases' idiomatic "platform admin only" pattern.
pub(crate) async fn admin_only(
    State(app_state): State<AppState>,
    AuthenticatedUser(user): AuthenticatedUser,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    let checks = vec![Predicate::new(UserIsAdmin, vec![])];
    authorize(&app_state, user, request, next, checks).await
}
