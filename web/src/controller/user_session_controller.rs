use crate::controller::ApiResponse;
use crate::error::{Error as WebError, Result as WebResult};
use axum::{http::StatusCode, response::IntoResponse, Form, Json};
use domain::user::{AuthSession, Credentials};
use log::*;
use serde_json::json;

/// Logs the user into the platform and returns a new session cookie.
///
/// Successful login will return a session cookie with id, e.g.:
/// set-cookie: id=07bbbe54-bd35-425f-8e63-618a8d8612df; HttpOnly; SameSite=Strict; Path=/; Max-Age=86399
///
/// After logging in successfully, you must pass the session id back to the server for
/// every API call, e.g.:
/// curl -v --header "Cookie: id=07bbbe54-bd35-425f-8e63-618a8d8612df" --request GET http://localhost:4000/organizations
#[utoipa::path(
    post,
    path = "/login",
    request_body(content = domain::user::Credentials, content_type = "application/x-www-form-urlencoded"),
    responses(
        (status = 200, description = "Logs in and returns session authentication cookie"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn login(
    mut auth_session: AuthSession,
    Form(creds): Form<Credentials>,
) -> WebResult<impl IntoResponse> {
    let user = match auth_session.authenticate(creds.clone()).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            // No user found - this should also be treated as an authentication error
            warn!("Authentication failed, invalid user: {:?}", creds.email);
            // TODO: replace this with a more idiomatic Rust 1-liner using from/into
            return Err(WebError::from(domain::error::Error {
                source: None,
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Unauthenticated,
                    ),
                ),
            }));
        }
        Err(auth_error) => {
            // Convert axum_login error to WebError by creating domain error manually.
            // This maps EntityApiErrorKind::RecordUnauthenticated to a 401 through the web layer.
            error!("Authentication failed with error: {auth_error:?}");
            // TODO: replace this with a more idiomatic Rust 1-liner using from/into
            return Err(WebError::from(domain::error::Error {
                source: Some(Box::new(auth_error)),
                error_kind: domain::error::DomainErrorKind::Internal(
                    domain::error::InternalErrorKind::Entity(
                        domain::error::EntityErrorKind::Unauthenticated,
                    ),
                ),
            }));
        }
    };

    if let Err(login_error) = auth_session.login(&user).await {
        warn!("Session login failed: {login_error:?}");
        return Err(WebError::from(domain::error::Error {
            source: Some(Box::new(login_error)),
            error_kind: domain::error::DomainErrorKind::Internal(
                domain::error::InternalErrorKind::Other("Session login failed".to_string()),
            ),
        }));
    }

    let user_session_json = json!({
            "id": user.id,
            "email": user.email,
            "first_name": user.first_name,
            "last_name": user.last_name,
            "display_name": user.display_name,
            "timezone": user.timezone,
            "role": user.role,
            "roles": user.roles
    });

    debug!("user_session_json: {user_session_json}");

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        user_session_json,
    )))
}

/// Logs the user out of the platform by destroying their session.
/// Test this with curl: curl -v \
/// --header "Cookie: id=07bbbe54-bd35-425f-8e63-618a8d8612df" \
/// --request DELETE http://localhost:4000/user_sessions/:id
#[utoipa::path(
get,
path = "/delete",
responses(
    (status = 200, description = "Successfully logged out"),
    (status = 401, description = "Unauthorized"),
    (status = 405, description = "Method not allowed"),
    (status = 503, description = "Service temporarily unavailable")
),
security(
    ("cookie_auth" = [])
)
)]
pub async fn delete(mut auth_session: AuthSession) -> impl IntoResponse {
    trace!("UserSessionController::delete()");
    match auth_session.logout().await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
