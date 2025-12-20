//! Controller for OAuth authentication flows.
//!
//! Handles Google OAuth for Google Meet integration.

use crate::extractors::authenticated_user::AuthenticatedUser;
use crate::extractors::compare_api_version::CompareApiVersion;
use crate::{AppState, Error};

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Redirect};

use domain::gateway::google_oauth::{GoogleOAuthClient, GoogleOAuthUrls};
use domain::user_integrations::Model as UserIntegrationModel;
use domain::{user_integration, Id};
use log::*;
use serde::Deserialize;
use service::config::ApiVersion;

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
#[utoipa::path(
    get,
    path = "/oauth/google/authorize",
    params(
        ApiVersion,
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
    CompareApiVersion(_v): CompareApiVersion,
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

    let urls = GoogleOAuthUrls {
        auth_url: config.google_oauth_auth_url().to_string(),
        token_url: config.google_oauth_token_url().to_string(),
        userinfo_url: config.google_userinfo_url().to_string(),
    };

    let client = GoogleOAuthClient::new(&client_id, "", &redirect_uri, urls)?;

    // Use user ID as state parameter for security
    let state = params.user_id.to_string();
    let auth_url = client.get_authorization_url(&state);

    info!("Redirecting user {} to Google OAuth", params.user_id);
    Ok(Redirect::temporary(&auth_url))
}

/// GET /oauth/google/callback
///
/// Handles the OAuth callback from Google after user authorization.
#[utoipa::path(
    get,
    path = "/oauth/google/callback",
    params(
        ApiVersion,
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
    CompareApiVersion(_v): CompareApiVersion,
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

    let urls = GoogleOAuthUrls {
        auth_url: config.google_oauth_auth_url().to_string(),
        token_url: config.google_oauth_token_url().to_string(),
        userinfo_url: config.google_userinfo_url().to_string(),
    };

    let client = GoogleOAuthClient::new(&client_id, &client_secret, &redirect_uri, urls)?;

    // Exchange authorization code for tokens
    let token_response = client.exchange_code(&params.code).await.map_err(|e| {
        warn!(
            "Failed to exchange OAuth code for user {}: {:?}",
            user_id, e
        );
        internal_error("Failed to complete Google authorization")
    })?;

    // Get user info from Google
    let user_info = client
        .get_user_info(&token_response.access_token)
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

    integration.google_access_token = Some(token_response.access_token);
    integration.google_refresh_token = token_response.refresh_token;
    integration.google_token_expiry =
        Some(chrono::Utc::now() + chrono::Duration::seconds(token_response.expires_in))
            .map(|dt| dt.into());
    integration.google_email = Some(user_info.email);

    let _updated: UserIntegrationModel =
        user_integration::update(app_state.db_conn_ref(), integration.id, integration).await?;

    info!(
        "Successfully stored Google OAuth tokens for user {}",
        user_id
    );

    // Redirect to settings page
    // The frontend URL should be configured, but we'll use a relative redirect for now
    let redirect_url = "/settings/integrations?google=connected".to_string();
    Ok(Redirect::temporary(&redirect_url))
}
