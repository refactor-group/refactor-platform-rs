use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::jwt as JwtApi;
use entity::Id;
use log::*;
use service::config::ApiVersion;

/// GET generate a collaboration token
#[utoipa::path(
    get,
    path = "/jwt/generate_collab_token",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Query, description = "Coaching session id to generate token for")
    ),
    responses(
        (status = 200, description = "Successfully generated a collaboration token", body = Jwt),  
        (status = 500, description = "Internal Server Error")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn generate_collab_token(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(coaching_session_id): Query<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "GET generate collaboration token for coaching session id: {}",
        coaching_session_id
    );

    let jwt = JwtApi::generate_collab_token(
        app_state.db_conn_ref(),
        &app_state.config,
        coaching_session_id,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), jwt)))
}
