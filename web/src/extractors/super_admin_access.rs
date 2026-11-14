use crate::extractors::{authenticated_user::AuthenticatedUser, RejectionType};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use domain::users;

/// Authorizes a system SuperAdmin (role `SuperAdmin` with no organization).
///
/// Carries the authenticated user so handlers can record who acted.
pub(crate) struct SuperAdminAccess {
    pub authenticated_user: users::Model,
}

#[async_trait]
impl<S> FromRequestParts<S> for SuperAdminAccess
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(authenticated_user) =
            AuthenticatedUser::from_request_parts(parts, state).await?;

        let is_super_admin = authenticated_user.is_super_admin();

        if is_super_admin {
            Ok(SuperAdminAccess { authenticated_user })
        } else {
            Err((
                StatusCode::FORBIDDEN,
                "SuperAdmin access required".to_string(),
            ))
        }
    }
}
