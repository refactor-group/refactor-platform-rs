//! Controller for OAuth authentication flows.
//!
//! Handles Google OAuth for Google Meet integration.
//!
//! Note: OAuth endpoints don't use CompareApiVersion because they work via
//! browser redirects which cannot set custom headers.

use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::{AppState, Error};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};

use domain::{oauth_connection, Id};
use serde::Deserialize;

use crate::error::WebErrorKind;

/// Query parameters for OAuth callback
#[derive(Debug, Deserialize)]
pub struct OAuthCallback {
    pub code: String,
    pub state: Option<String>,
}

/// Query parameters for starting OAuth
#[derive(Debug, Deserialize)]
pub struct OAuthStart {
    pub user_id: Id,
}

/// GET /oauth/google/authorize
///
/// Initiates Google OAuth flow by redirecting to Google's authorization endpoint.
/// Note: This endpoint doesn't require x-version header as it's called via browser redirect.
#[utoipa::path(
    get,
    path = "/oauth/google/authorize",
    params(
        ("user_id" = Id, Query, description = "User ID to associate with Google account"),
    ),
    responses(
        (status = 302, description = "Redirect to Google OAuth"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Server error (OAuth not configured)"),
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn authorize(
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Query(params): Query<OAuthStart>,
) -> Result<impl IntoResponse, Error> {
    if user.id != params.user_id {
        return Err(Error::Web(WebErrorKind::Auth));
    }

    let url = oauth_connection::google_authorize_url(&app_state.config, params.user_id)?;
    Ok(Redirect::temporary(&url))
}

/// GET /oauth/google/callback
///
/// Handles the OAuth callback from Google after user authorization.
/// Note: This endpoint doesn't require x-version header as it's called via Google's redirect.
#[utoipa::path(
    get,
    path = "/oauth/google/callback",
    params(
        ("code" = String, Query, description = "Authorization code from Google"),
        ("state" = Option<String>, Query, description = "State parameter (user ID)"),
    ),
    responses(
        (status = 302, description = "Redirect to settings page on success"),
        (status = 400, description = "Invalid callback parameters"),
        (status = 500, description = "Token exchange failed"),
    )
)]
pub async fn callback(
    State(app_state): State<AppState>,
    Query(params): Query<OAuthCallback>,
) -> Result<impl IntoResponse, Error> {
    let user_id: Id = params
        .state
        .as_ref()
        .ok_or(Error::Web(WebErrorKind::Input))?
        .parse()
        .map_err(|_| Error::Web(WebErrorKind::Input))?;

    let redirect_url = oauth_connection::exchange_and_store_tokens(
        app_state.db_conn_ref(),
        &app_state.config,
        user_id,
        &params.code,
    )
    .await?;

    Ok(Redirect::temporary(&redirect_url))
}
