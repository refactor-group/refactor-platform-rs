use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use domain::Id;
use log::*;

use crate::extractors::{
    authenticated_user::AuthenticatedUser, parse_path_id_from_parts, RejectionType,
};

/// Admits only the `:user_id` user themselves, yielding that id.
pub(crate) struct UserSelfAccess(pub Id);

#[async_trait]
impl<S> FromRequestParts<S> for UserSelfAccess
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let AuthenticatedUser(user) = AuthenticatedUser::from_request_parts(parts, state).await?;

        let user_id = parse_path_id_from_parts(parts, "user_id").await?;

        if user.id == user_id {
            return Ok(UserSelfAccess(user_id));
        }

        warn!("Forbidden: user {} cannot act as user {user_id}", user.id);
        Err((StatusCode::FORBIDDEN, "FORBIDDEN".to_string()))
    }
}
