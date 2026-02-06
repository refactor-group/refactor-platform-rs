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

use domain::gateway::oauth::{self, Provider};
use domain::user_integrations::Model as UserIntegrationModel;
use domain::{user_integration, Id};
use log::*;
use secrecy::ExposeSecret;
use serde::Deserialize;

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

/// Helper to create an internal server error
fn internal_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Other(message.to_string()),
        ),
    })
}

/// Helper to create a forbidden error
fn forbidden_error(message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Other(
                message.to_string(),
            )),
        ),
    })
}

/// Helper to create a bad request error
fn bad_request_error(_message: &str) -> Error {
    Error::Domain(domain::error::Error {
        source: None,
        error_kind: domain::error::DomainErrorKind::Internal(
            domain::error::InternalErrorKind::Entity(domain::error::EntityErrorKind::Invalid),
        ),
    })
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
    // Verify user is authorizing their own account
    if user.id != params.user_id {
        return Err(forbidden_error("Cannot authorize OAuth for another user"));
    }

    let config = &app_state.config;

    // Check if Google OAuth is configured
    let client_id = config.google_client_id().ok_or_else(|| {
        warn!("Google OAuth not configured: missing client_id");
        internal_error("Google OAuth not configured")
    })?;

    let redirect_uri = config.google_redirect_uri().ok_or_else(|| {
        warn!("Google OAuth not configured: missing redirect_uri");
        internal_error("Google OAuth not configured")
    })?;

    // Create Google OAuth provider (client_secret not needed for authorization URL)
    let provider = oauth::google::new_provider(client_id, String::new(), redirect_uri);

    // Use user ID as state parameter for security
    let state = params.user_id.to_string();
    let auth_request = provider.authorization_url(&state, None);

    info!("Redirecting user {} to Google OAuth", params.user_id);
    Ok(Redirect::temporary(&auth_request.url))
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
    let config = &app_state.config;

    // Extract user ID from state
    let user_id: Id = params
        .state
        .as_ref()
        .ok_or_else(|| bad_request_error("Missing state parameter"))?
        .parse()
        .map_err(|_| bad_request_error("Invalid state parameter"))?;

    info!("Processing Google OAuth callback for user {}", user_id);

    // Get OAuth configuration
    let client_id = config
        .google_client_id()
        .ok_or_else(|| internal_error("Google OAuth not configured"))?;

    let client_secret = config
        .google_client_secret()
        .ok_or_else(|| internal_error("Google OAuth not configured"))?;

    let redirect_uri = config
        .google_redirect_uri()
        .ok_or_else(|| internal_error("Google OAuth not configured"))?;

    // Create Google OAuth provider
    let provider = oauth::google::new_provider(client_id, client_secret, redirect_uri);

    // Exchange authorization code for tokens
    let tokens = provider.exchange_code(&params.code, None).await.map_err(|e| {
        warn!(
            "Failed to exchange OAuth code for user {}: {:?}",
            user_id, e
        );
        internal_error("Failed to complete Google authorization")
    })?;

    // Get user info from Google
    let user_info = provider
        .get_user_info(tokens.access_token.expose_secret())
        .await
        .map_err(|e| {
            warn!(
                "Failed to get Google user info for user {}: {:?}",
                user_id, e
            );
            internal_error("Failed to get Google user info")
        })?;

    // Store tokens in user integrations
    let mut integration: UserIntegrationModel =
        user_integration::get_or_create(app_state.db_conn_ref(), user_id).await?;

    integration.google_access_token = Some(tokens.access_token.expose_secret().to_string());
    integration.google_refresh_token = tokens.refresh_token.map(|rt| rt.expose_secret().to_string());
    integration.google_token_expiry = tokens.expires_at.map(|dt| dt.into());
    integration.google_email = Some(user_info.email);

    let _updated: UserIntegrationModel =
        user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;

    info!(
        "Successfully stored Google OAuth tokens for user {}",
        user_id
    );

    // Redirect to frontend settings page
    let base_url = config.google_oauth_success_redirect_uri();
    let redirect_url = format!("{}?google=connected", base_url);
    Ok(Redirect::temporary(&redirect_url))
}
