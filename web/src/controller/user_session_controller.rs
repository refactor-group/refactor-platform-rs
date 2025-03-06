use crate::controller::ApiResponse;
use axum::{http::StatusCode, response::IntoResponse, Form, Json};
use domain::user::{AuthSession, Credentials};
use log::*;
use serde::Deserialize;
use serde_json::json;

// This allows us to extract the "next" field from the query string. We use this
// to redirect after log in.
#[derive(Debug, Deserialize)]
pub struct NextUrl {
    _next: Option<String>,
}

pub async fn protected(auth_session: AuthSession) -> impl IntoResponse {
    debug!("UserSessionController::protected()");

    match auth_session.user {
        Some(user) => json!({
            "email": &user.email,
        })
        .to_string()
        .into_response(),

        None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

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
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn login(
    mut auth_session: AuthSession,
    Form(creds): Form<Credentials>,
) -> impl IntoResponse {
    debug!("UserSessionController::login()");
    let user = match auth_session.authenticate(creds.clone()).await {
        Ok(Some(user)) => user,
        Ok(None) => return Err(StatusCode::UNAUTHORIZED.into_response()),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    };

    if auth_session.login(&user).await.is_err() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    }

    let user_session_json = json!({
            "id": user.id,
            "email": user.email,
            "first_name": user.first_name,
            "last_name": user.last_name,
            "display_name": user.display_name,
    });

    debug!("user_session_json: {}", user_session_json);

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        user_session_json,
    )))
}

/// Logs the user out of the platform by destroying their session.
/// Test this with curl: curl -v \
/// --header "Cookie: id=07bbbe54-bd35-425f-8e63-618a8d8612df" \
/// --request GET http://localhost:4000/logout
#[utoipa::path(
get,
path = "/logout",
responses(
    (status = 200, description = "Successfully logged out"),
    (status = 401, description = "Unauthorized"),
    (status = 405, description = "Method not allowed")
),
security(
    ("cookie_auth" = [])
)
)]
pub async fn logout(mut auth_session: AuthSession) -> impl IntoResponse {
    debug!("UserSessionController::logout()");
    match auth_session.logout().await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
