use crate::{controller::ApiResponse, params::user::CompleteSetupParams, AppState, Error};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::magic_link_token::{self as MagicLinkTokenApi};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub token: String,
}

/// GET /magic-link/validate
///
/// Validate a magic link token without consuming it.
///
/// Returns the user's profile data so the frontend can pre-fill the setup form.

#[utoipa::path(
    get,
    path = "/magic-link/validate",
    params(
        ("token" = String, Query, description = "Magic login token from the welcome email"),
    ),
    responses(
        (status = 200, description = "Successfully retrieved a User", body = User),
        (status = 400, description = "Invalid login token"),
        (status = 401, description = "Expired token"),
    )
)]
pub(crate) async fn validate(
    State(app_state): State<AppState>,
    Query(params): Query<ValidateParams>,
) -> Result<impl IntoResponse, Error> {
    let user = MagicLinkTokenApi::validate_token(app_state.db_conn_ref(), &params.token).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
}

/// POST /magic-link/complete-setup
///
/// Consume a magic link token and complete user account setup.
///
/// Sets the user's password and optionally updates profile fields.
/// The token is deleted after successful consumption.
///
#[utoipa::path(
    post,
    path = "/magic-link/complete-setup",
    params(
        ("token" = String, Query, description = "Magic login token from the welcome email"),
    ),
    request_body = CompleteSetupParams,
    responses(
        (status = 200, description = "User profile successfully updated", body = User),
        (status = 400, description = "Password confirmation does not match"),
        (status = 503, description = "Service temporarily unavailable")
    )
)]
pub(crate) async fn complete_setup(
    State(app_state): State<AppState>,
    Json(params): Json<CompleteSetupParams>,
) -> Result<impl IntoResponse, Error> {
    let updated_user = MagicLinkTokenApi::complete_setup(app_state.db_conn_ref(), params).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_user)))
}
