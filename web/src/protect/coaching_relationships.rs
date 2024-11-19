use axum::extract::Request;
use entity_api::user::AuthSession;

struct AuthorizationError {}

pub(crate) async fn protect(
    auth_session: AuthSession,
    request: Request,
) -> Result<Request, AuthorizationError> {
    // here we have access to the current user (actor) making the request
    // as well as the request itself (from which we can determine which resource is being acted upon).
    // We can use both pieces of information to determine if the actor is allowed to operate on the resource.
    Ok(request)
}
