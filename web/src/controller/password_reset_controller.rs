//! Handlers for user-initiated password reset.
//!
//! See `docs/architecture/password_reset.md` for the full design and threat
//! model. Three unauthenticated endpoints:
//!
//! - `POST /password-reset/request` — always returns 200 (enumeration-safe).
//! - `GET  /password-reset/validate?token=<raw>` — non-destructive validity check.
//! - `POST /password-reset/complete` — consume the token and set the new password.

use crate::{
    controller::ApiResponse,
    params::user::{PasswordResetCompleteParams, PasswordResetRequestParams},
    AppState, Error,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use domain::password_reset as PasswordResetApi;
use log::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize)]
pub(crate) struct ValidateParams {
    pub token: String,
}

/// Sanitized user data returned by the validate endpoint.
///
/// Deliberately narrow: only what the FE needs to render a personalized
/// "Hi <first_name>, set your new password" form. No email, role,
/// organization membership, or other PII.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct ValidateResponse {
    pub first_name: String,
    pub last_name: String,
}

/// POST /password-reset/request
///
/// Send a password-reset email if the email maps to a real user.
/// **Always returns 200** regardless of whether the user exists
/// (enumeration-safe). A constant-time padding step in the domain layer
/// defeats the timing-based variant of the same attack.
#[utoipa::path(
    post,
    path = "/password-reset/request",
    request_body = PasswordResetRequestParams,
    responses(
        (status = 200, description = "Request accepted (returned whether or not the email exists)"),
        (status = 429, description = "Per-email rate limit exceeded"),
        (status = 503, description = "Service temporarily unavailable"),
    )
)]
pub(crate) async fn request(
    State(app_state): State<AppState>,
    Json(params): Json<PasswordResetRequestParams>,
) -> Result<impl IntoResponse, Error> {
    warn!("[password-reset] /request endpoint hit");
    debug!("[password-reset] /request raw email: {}", params.email);

    PasswordResetApi::request_password_reset(
        std::sync::Arc::clone(&app_state.database_connection),
        &params.email,
        &app_state.config,
    )
    .await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        serde_json::Value::Null,
    )))
}

/// GET /password-reset/validate
///
/// Validate a password-reset token without consuming it. Returns sanitized
/// user data (first/last name only) so the FE can render a personalized
/// form. Maps any underlying validation failure to the collapsed
/// `400 invalid_or_expired_token` response — the FE cannot distinguish
/// "never existed" from "expired" from "wrong purpose."
#[utoipa::path(
    get,
    path = "/password-reset/validate",
    params(
        ("token" = String, Query, description = "Password reset token from the email"),
    ),
    responses(
        (status = 200, description = "Token valid; returns sanitized user data", body = ValidateResponse),
        (status = 400, description = "Token invalid, expired, or wrong purpose"),
        (status = 503, description = "Service temporarily unavailable"),
    )
)]
pub(crate) async fn validate(
    State(app_state): State<AppState>,
    Query(params): Query<ValidateParams>,
) -> Result<impl IntoResponse, Error> {
    warn!("[password-reset] /validate endpoint hit");

    let user =
        PasswordResetApi::validate_reset_token(app_state.db_conn_ref(), &params.token).await?;

    let body = ValidateResponse {
        first_name: user.first_name,
        last_name: user.last_name,
    };

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), body)))
}

/// POST /password-reset/complete
///
/// Consume a password-reset token and set the user's new password. On
/// success the token is deleted (single-use) and the full updated user
/// is returned. The FE should redirect to the login page; this endpoint
/// does **not** auto-log-in.
#[utoipa::path(
    post,
    path = "/password-reset/complete",
    request_body = PasswordResetCompleteParams,
    responses(
        (status = 200, description = "Password updated; user can now log in with the new password", body = domain::users::Model),
        (status = 400, description = "Token invalid, expired, or wrong purpose"),
        (status = 422, description = "Password confirmation does not match"),
        (status = 503, description = "Service temporarily unavailable"),
    )
)]
pub(crate) async fn complete(
    State(app_state): State<AppState>,
    Json(params): Json<PasswordResetCompleteParams>,
) -> Result<impl IntoResponse, Error> {
    warn!("[password-reset] /complete endpoint hit");

    let updated_user =
        PasswordResetApi::complete_password_reset(app_state.db_conn_ref(), params).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_user)))
}
