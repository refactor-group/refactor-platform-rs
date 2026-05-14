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
    params::{
        user::{PasswordResetCompleteParams, PasswordResetRequestParams},
        validation::{validate_email_shape, validate_token_length},
    },
    AppState, Error,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use domain::password_reset as PasswordResetApi;
use log::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
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

    // Reject malformed/oversized email at the HTTP boundary before any
    // expensive work (SHA-256 hash, DB query). Returns 400. See
    // `crate::params::validation::validate_email_shape`.
    validate_email_shape(&params.email)?;

    // NEVER log the raw email at any level. Even DEBUG is too risky
    // because ops teams enable DEBUG during incidents, log aggregators
    // may retain DEBUG longer than WARN, and access controls are coarser
    // than "by log level." The hash-prefix gives operators correlation
    // capability without plaintext exposure.

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

/// POST /password-reset/validate
///
/// Validate a password-reset token without consuming it. Returns sanitized
/// user data (first/last name only) so the FE can render a personalized
/// form. Maps any underlying validation failure to the collapsed
/// `400 invalid_or_expired_token` response — the FE cannot distinguish
/// "never existed" from "expired" from "wrong purpose."
///
/// **Token is in the JSON body, NOT a URL query parameter** (contract v1.1
/// change from v1). Query-string tokens land in access logs, browser
/// history, and reverse-proxy logs — body-transport keeps them off all
/// those channels. See `PasswordResetEndpoints` v1.1 contract on the
/// coordinator blackboard and the FE-raised question
/// `password_reset_validate_token_transport`.
#[utoipa::path(
    post,
    path = "/password-reset/validate",
    request_body = ValidateParams,
    responses(
        (status = 200, description = "Token valid; returns sanitized user data", body = ValidateResponse),
        (status = 400, description = "Token invalid, expired, or wrong purpose"),
        (status = 503, description = "Service temporarily unavailable"),
    )
)]
pub(crate) async fn validate(
    State(app_state): State<AppState>,
    Json(params): Json<ValidateParams>,
) -> Result<impl IntoResponse, Error> {
    warn!("[password-reset] /validate endpoint hit");

    // Reject wrong-length tokens at the HTTP boundary before paying for
    // SHA-256 + DB lookup. Tokens we issue are always 43 chars.
    validate_token_length(&params.token)?;

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

    // Reject wrong-length tokens at the HTTP boundary (cheap before the
    // DB-bound complete flow runs).
    validate_token_length(&params.token)?;

    let updated_user =
        PasswordResetApi::complete_password_reset(app_state.db_conn_ref(), params).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), updated_user)))
}
