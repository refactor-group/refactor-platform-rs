use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, coaching_session_access::CoachingSessionQueryAccess,
    compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::jwt as JwtApi;
use log::*;
use service::config::ApiVersion;

/// GET generate a collaboration token
#[utoipa::path(
    get,
    path = "/jwt/generate_collab_token",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Query, description = "Coaching session id to generate token for")
    ),
    responses(
        (status = 200, description = "Successfully generated a collaboration token", body = domain::jwts::Jwt),  
        (status = 403, description = "Forbidden"),
        (status = 500, description = "Internal Server Error"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn generate_collab_token(
    CoachingSessionQueryAccess(session, _relationship): CoachingSessionQueryAccess,
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET generate collaboration token for coaching session id: {}",
        session.id
    );

    let jwt = JwtApi::generate_collab_token(app_state.db_conn_ref(), &app_state.config, session.id)
        .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), jwt)))
}

#[cfg(test)]
#[path = "jwt_controller_sqlite_tests.rs"]
mod sqlite_tests;
