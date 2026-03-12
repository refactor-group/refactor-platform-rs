//! Controller for OAuth authentication flows and connection management.
//!
//! Handles Google OAuth for Google Meet integration.
//!
//! Note: The authorize/callback endpoints don't use CompareApiVersion because they work via
//! browser redirects which cannot set custom headers.

use crate::{AppState, Error};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};

use domain::{oauth_connection, Id};
use serde::{Deserialize};

use crate::error::WebErrorKind;

/// Query parameters for OAuth callback
#[derive(Debug, Deserialize)]
pub struct OAuthCallback {
    pub code: String,
    pub state: Option<String>,
}

/// GET /oauth/google/callback
///
/// Handles the OAuth callback from a provider after user authorization.
/// Note: This endpoint doesn't require x-version header as it's called via the provider's redirect.
#[utoipa::path(
    get,
    path = "/oauth/google/callback",
    params(
        ("code" = String, Query, description = "Authorization code from Google"),
        ("state" = Option<String>, Query, description = "CSRF state token"),
    ),
    responses(
        (status = 302, description = "Redirect to settings page on success"),
        (status = 400, description = "Invalid callback parameters"),
        (status = 500, description = "Token exchange failed"),
    )
)]
pub async fn google_callback(
    State(app_state): State<AppState>,
    Query(params): Query<OAuthCallback>,
) -> Result<impl IntoResponse, Error> {
    let state_token = params
        .state
        .as_deref()
        .ok_or(Error::Web(WebErrorKind::Input))?;

    let state_data = app_state
        .oauth_state_manager
        .validate(state_token)
        .ok_or(Error::Web(WebErrorKind::Input))?;

    let user_id: Id = state_data
        .metadata
        .get("user_id")
        .ok_or(Error::Web(WebErrorKind::Input))?
        .parse()
        .map_err(|_| Error::Web(WebErrorKind::Input))?;

    let redirect_url = oauth_connection::exchange_and_store_google_tokens(
        app_state.db_conn_ref(),
        &app_state.config,
        user_id,
        &params.code,
    )
    .await?;

    Ok(Redirect::temporary(&redirect_url))
}

/// GET /oauth/zoom/callback
///
/// Handles the OAuth callback from Zoom after user authorization.
/// Note: This endpoint doesn't require x-version header as it's called via Zoom's redirect.
#[utoipa::path(
    get,
    path = "/oauth/zoom/callback",
    params(
        ("code" = String, Query, description = "Authorization code from Zoom"),
        ("state" = Option<String>, Query, description = "CSRF state token"),
    ),
    responses(
        (status = 302, description = "Redirect to settings page on success"),
        (status = 400, description = "Invalid callback parameters"),
        (status = 500, description = "Token exchange failed"),
    )
)]
pub async fn zoom_callback(
    State(app_state): State<AppState>,
    Query(params): Query<OAuthCallback>,
) -> Result<impl IntoResponse, Error> {
    let state_token = params
        .state
        .as_deref()
        .ok_or(Error::Web(WebErrorKind::Input))?;

    let state_data = app_state
        .oauth_state_manager
        .validate(state_token)
        .ok_or(Error::Web(WebErrorKind::Input))?;

    let user_id: Id = state_data
        .metadata
        .get("user_id")
        .ok_or(Error::Web(WebErrorKind::Input))?
        .parse()
        .map_err(|_| Error::Web(WebErrorKind::Input))?;

    let redirect_url = oauth_connection::exchange_and_store_zoom_tokens(
        app_state.db_conn_ref(),
        &app_state.config,
        user_id,
        &params.code,
    )
    .await?;

    Ok(Redirect::temporary(&redirect_url))
}
