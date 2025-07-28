use crate::extractors::RejectionType;
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{request::Parts, StatusCode},
};
use axum_login::AuthSession;
use domain::users;
use log::*;
use tower_sessions::Session;

pub(crate) struct AuthenticatedUser(pub users::Model);

#[async_trait]
impl<S> FromRequestParts<S> for AuthenticatedUser
where
    S: Send + Sync,
{
    type Rejection = RejectionType;

    // This extractor wraps the AuthSession extractor from axum_login. It extracts the user from the AuthSession and returns an AuthenticatedUser.
    // If the user is authenticated. If the user is not authenticated, it returns an Unauthorized error.
    // Additionally, it touches the session to update the last activity timestamp for session renewal.
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session: domain::user::AuthSession = AuthSession::from_request_parts(parts, state)
            .await
            .map_err(|(status, msg)| (status, msg.to_string()))?;
        
        // Touch the session to update activity timestamp for session renewal
        if let Ok(tower_session) = Session::from_request_parts(parts, state).await {
            if let Err(e) = tower_session.save().await {
                warn!("Failed to touch session for activity renewal: {:?}", e);
                // Continue with authentication - session touch failure shouldn't block authentication
            } else {
                trace!("Session touched successfully for activity renewal");
            }
        }
        
        match session.user {
            Some(user) => Ok(AuthenticatedUser(user)),
            None => Err((StatusCode::UNAUTHORIZED, "Unauthorized".to_string())),
        }
    }
}
