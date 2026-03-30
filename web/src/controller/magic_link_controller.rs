use crate::{controller::ApiResponse, AppState, Error};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::magic_link_token::{self as MagicLinkTokenApi, SetupProfile};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub token: String,
}

/// Validate a magic link token without consuming it.
///
/// Returns the user's profile data so the frontend can pre-fill the setup form.
pub(crate) async fn validate(
    State(app_state): State<AppState>,
    Query(params): Query<ValidateParams>,
) -> Result<impl IntoResponse, Error> {
    let user = MagicLinkTokenApi::validate_token(app_state.db_conn_ref(), &params.token).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), user)))
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompleteSetupParams {
    pub token: String,
    pub password: String,
    pub confirm_password: String,
    pub display_name: Option<String>,
    pub github_username: Option<String>,
    pub github_profile_url: Option<String>,
    pub timezone: Option<String>,
}

/// Consume a magic link token and complete user account setup.
///
/// Sets the user's password and optionally updates profile fields.
/// The token is deleted after successful consumption.
pub(crate) async fn complete_setup(
    State(app_state): State<AppState>,
    Json(params): Json<CompleteSetupParams>,
) -> Result<impl IntoResponse, Error> {
    let profile = SetupProfile {
        display_name: params.display_name,
        github_username: params.github_username,
        github_profile_url: params.github_profile_url,
        timezone: params.timezone,
    };

    let updated_user = MagicLinkTokenApi::complete_setup(
        app_state.db_conn_ref(),
        &params.token,
        params.password,
        params.confirm_password,
        profile,
    )
    .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_user)))
}
