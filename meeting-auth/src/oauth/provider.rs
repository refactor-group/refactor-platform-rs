//! OAuth provider trait and types.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::token::{RefreshResult, Tokens};
use crate::error::Error;

/// Known OAuth providers for video meetings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Google,
    Zoom,
    Microsoft,
}

impl ProviderKind {
    /// Get the provider identifier string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Google => "google",
            ProviderKind::Zoom => "zoom",
            ProviderKind::Microsoft => "microsoft",
        }
    }
}

/// Authorization request with URL and state management data.
#[derive(Debug, Clone)]
pub struct AuthorizationRequest {
    /// Authorization URL to redirect the user to.
    pub url: String,
    /// CSRF state parameter for validation.
    pub state: String,
    /// PKCE verifier to be stored for later code exchange.
    pub pkce_verifier: Option<String>,
}

/// User information retrieved from OAuth provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    /// Provider's unique user identifier.
    pub id: String,
    /// User's email address.
    pub email: String,
    /// User's display name.
    pub name: Option<String>,
    /// User's profile picture URL.
    pub picture: Option<String>,
    /// Whether the email is verified.
    pub email_verified: Option<bool>,
}

/// Trait for OAuth 2.0 providers.
///
/// Implementations handle platform-specific OAuth flows including:
/// - Authorization URL generation with PKCE
/// - Authorization code exchange for tokens
/// - Token refresh (including rotating refresh tokens for Zoom)
/// - Token revocation
/// - User info retrieval
#[async_trait]
pub trait Provider: Send + Sync {
    /// Get the provider kind.
    fn provider(&self) -> ProviderKind;

    /// Generate authorization URL with state and optional PKCE challenge.
    ///
    /// # Arguments
    ///
    /// * `state` - CSRF state parameter for validation
    /// * `pkce_challenge` - Optional PKCE code challenge
    ///
    /// # Returns
    ///
    /// Authorization request containing the URL, state, and PKCE verifier.
    fn authorization_url(&self, state: &str, pkce_challenge: Option<&str>) -> AuthorizationRequest;

    /// Exchange authorization code for access and refresh tokens.
    ///
    /// # Arguments
    ///
    /// * `code` - Authorization code from OAuth callback
    /// * `pkce_verifier` - PKCE code verifier if PKCE was used
    ///
    /// # Returns
    ///
    /// OAuth tokens including access token, refresh token, and expiry.
    async fn exchange_code(&self, code: &str, pkce_verifier: Option<&str>)
        -> Result<Tokens, Error>;

    /// Refresh an access token using a refresh token.
    ///
    /// # Arguments
    ///
    /// * `refresh_token` - The refresh token
    ///
    /// # Returns
    ///
    /// Refresh result with new tokens and indication if refresh token rotated.
    async fn refresh_token(&self, refresh_token: &str) -> Result<RefreshResult, Error>;

    /// Revoke a token (access or refresh).
    ///
    /// # Arguments
    ///
    /// * `token` - The token to revoke
    async fn revoke_token(&self, token: &str) -> Result<(), Error>;

    /// Get user information using an access token.
    ///
    /// # Arguments
    ///
    /// * `access_token` - Valid access token
    ///
    /// # Returns
    ///
    /// User information from the OAuth provider.
    async fn get_user_info(&self, access_token: &str) -> Result<UserInfo, Error>;

    /// Returns true if this provider rotates refresh tokens (e.g., Zoom).
    ///
    /// When true, the TokenManager will use atomic updates to handle token rotation.
    fn uses_rotating_refresh_tokens(&self) -> bool {
        false
    }
}
