//! Google OAuth provider implementation.

use async_trait::async_trait;

use crate::error::{oauth_error, Error, OAuthErrorKind};
use crate::oauth::{AuthorizationRequest, ProviderKind, UserInfo};
use crate::oauth::token::{RefreshResult, Tokens};

/// Google OAuth provider.
///
/// Handles OAuth 2.0 flows for Google accounts, including:
/// - Authorization URL generation with PKCE
/// - Authorization code exchange
/// - Token refresh
/// - User info retrieval from Google APIs
pub struct Provider {
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    http_client: reqwest::Client,
}

impl Provider {
    /// Create a new Google OAuth provider.
    ///
    /// # Arguments
    ///
    /// * `client_id` - Google OAuth client ID
    /// * `client_secret` - Google OAuth client secret
    /// * `redirect_uri` - OAuth redirect URI
    pub fn new(client_id: String, client_secret: String, redirect_uri: String) -> Self {
        Self {
            client_id,
            client_secret,
            redirect_uri,
            http_client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl crate::oauth::Provider for Provider {
    fn provider(&self) -> ProviderKind {
        ProviderKind::Google
    }

    fn authorization_url(&self, state: &str, _pkce_challenge: Option<&str>) -> AuthorizationRequest {
        // TODO: Implement Google authorization URL generation
        // Reference: domain/src/gateway/google_oauth.rs
        AuthorizationRequest {
            url: String::new(),
            state: state.to_string(),
            pkce_verifier: None,
        }
    }

    async fn exchange_code(&self, _code: &str, _pkce_verifier: Option<&str>) -> Result<Tokens, Error> {
        // TODO: Implement Google code exchange
        // Reference: domain/src/gateway/google_oauth.rs
        Err(oauth_error(
            OAuthErrorKind::TokenExchangeFailed,
            "Not yet implemented",
        ))
    }

    async fn refresh_token(&self, _refresh_token: &str) -> Result<RefreshResult, Error> {
        // TODO: Implement Google token refresh
        // Reference: domain/src/gateway/google_oauth.rs
        Err(oauth_error(
            OAuthErrorKind::TokenRefreshFailed,
            "Not yet implemented",
        ))
    }

    async fn revoke_token(&self, _token: &str) -> Result<(), Error> {
        // TODO: Implement Google token revocation
        Err(oauth_error(
            OAuthErrorKind::RevocationFailed,
            "Not yet implemented",
        ))
    }

    async fn get_user_info(&self, _access_token: &str) -> Result<UserInfo, Error> {
        // TODO: Implement Google user info retrieval
        // Reference: domain/src/gateway/google_oauth.rs
        Err(oauth_error(
            OAuthErrorKind::InvalidResponse,
            "Not yet implemented",
        ))
    }

    fn uses_rotating_refresh_tokens(&self) -> bool {
        false // Google does not rotate refresh tokens
    }
}
