//! Google OAuth provider implementation.

use async_trait::async_trait;
use chrono::Utc;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::error::{oauth_error, Error, OAuthErrorKind};
use crate::oauth::token::{RefreshResult, Tokens};
use crate::oauth::{AuthorizationRequest, ProviderKind, UserInfo as OAuthUserInfo};

/// Google OAuth endpoints.
const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

/// OAuth scopes for Google Meet and user profile access.
const SCOPES: &[&str] = &[
    "openid",
    "email",
    "profile",
    "https://www.googleapis.com/auth/meetings.space.created",
];

/// Token exchange request.
#[derive(Debug, Serialize)]
struct TokenExchangeRequest {
    code: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    grant_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    code_verifier: Option<String>,
}

/// Token refresh request.
#[derive(Debug, Serialize)]
struct TokenRefreshRequest {
    refresh_token: String,
    client_id: String,
    client_secret: String,
    grant_type: String,
}

/// Token response from Google.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    expires_in: i64,
    token_type: String,
    #[serde(default)]
    scope: String,
}

/// User info response from Google.
#[derive(Debug, Deserialize)]
struct UserInfo {
    id: String,
    email: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
    #[serde(default)]
    verified_email: Option<bool>,
}

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

    fn authorization_url(&self, state: &str, pkce_challenge: Option<&str>) -> AuthorizationRequest {
        let scopes = SCOPES.join(" ");

        let mut url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent&state={}",
            AUTH_URL,
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&scopes),
            urlencoding::encode(state)
        );

        // Add PKCE challenge if provided
        if let Some(challenge) = pkce_challenge {
            url.push_str("&code_challenge=");
            url.push_str(&urlencoding::encode(challenge));
            url.push_str("&code_challenge_method=S256");
        }

        AuthorizationRequest {
            url,
            state: state.to_string(),
            pkce_verifier: None, // Verifier is managed by caller
        }
    }

    async fn exchange_code(
        &self,
        code: &str,
        pkce_verifier: Option<&str>,
    ) -> Result<Tokens, Error> {
        let request = TokenExchangeRequest {
            code: code.to_string(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_uri: self.redirect_uri.clone(),
            grant_type: "authorization_code".to_string(),
            code_verifier: pkce_verifier.map(|s| s.to_string()),
        };

        debug!("Exchanging Google OAuth code for tokens");

        let response = self
            .http_client
            .post(TOKEN_URL)
            .form(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to exchange Google OAuth code: {:?}", e);
                oauth_error(OAuthErrorKind::Network, &e.to_string())
            })?;

        if response.status().is_success() {
            let token_response: TokenResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Google token response: {:?}", e);
                oauth_error(OAuthErrorKind::InvalidResponse, "Invalid token response")
            })?;

            let expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in);
            let scopes: Vec<String> = token_response
                .scope
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();

            info!("Successfully exchanged Google OAuth code for tokens");

            Ok(Tokens {
                access_token: SecretString::from(token_response.access_token),
                refresh_token: token_response.refresh_token.map(SecretString::from),
                expires_at: Some(expires_at),
                token_type: token_response.token_type,
                scopes,
            })
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google OAuth error: {}", error_text);
            Err(oauth_error(
                OAuthErrorKind::TokenExchangeFailed,
                &error_text,
            ))
        }
    }

    async fn refresh_token(&self, refresh_token: &str) -> Result<RefreshResult, Error> {
        let request = TokenRefreshRequest {
            refresh_token: refresh_token.to_string(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            grant_type: "refresh_token".to_string(),
        };

        debug!("Refreshing Google access token");

        let response = self
            .http_client
            .post(TOKEN_URL)
            .form(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to refresh Google token: {:?}", e);
                oauth_error(OAuthErrorKind::Network, &e.to_string())
            })?;

        if response.status().is_success() {
            let token_response: TokenResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Google token refresh response: {:?}", e);
                oauth_error(OAuthErrorKind::InvalidResponse, "Invalid refresh response")
            })?;

            let expires_at = Utc::now() + chrono::Duration::seconds(token_response.expires_in);
            let scopes: Vec<String> = token_response
                .scope
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();

            info!("Successfully refreshed Google access token");

            let tokens = Tokens {
                access_token: SecretString::from(token_response.access_token),
                refresh_token: token_response
                    .refresh_token
                    .map(SecretString::from)
                    .or_else(|| Some(SecretString::from(refresh_token.to_string()))),
                expires_at: Some(expires_at),
                token_type: token_response.token_type,
                scopes,
            };

            Ok(RefreshResult::no_rotation(tokens))
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google token refresh error: {}", error_text);
            Err(oauth_error(OAuthErrorKind::TokenRefreshFailed, &error_text))
        }
    }

    async fn revoke_token(&self, token: &str) -> Result<(), Error> {
        debug!("Revoking Google token");

        let response = self
            .http_client
            .post(REVOKE_URL)
            .form(&[("token", token)])
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to revoke Google token: {:?}", e);
                oauth_error(OAuthErrorKind::Network, &e.to_string())
            })?;

        if response.status().is_success() {
            info!("Successfully revoked Google token");
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google token revocation error: {}", error_text);
            Err(oauth_error(OAuthErrorKind::RevocationFailed, &error_text))
        }
    }

    async fn get_user_info(&self, access_token: &str) -> Result<OAuthUserInfo, Error> {
        debug!("Fetching Google user info");

        let response = self
            .http_client
            .get(USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to get Google user info: {:?}", e);
                oauth_error(OAuthErrorKind::Network, &e.to_string())
            })?;

        if response.status().is_success() {
            let user_info: UserInfo = response.json().await.map_err(|e| {
                warn!("Failed to parse Google user info: {:?}", e);
                oauth_error(
                    OAuthErrorKind::InvalidResponse,
                    "Invalid user info response",
                )
            })?;

            info!("Successfully retrieved Google user info");

            Ok(OAuthUserInfo {
                id: user_info.id,
                email: user_info.email,
                name: user_info.name,
                picture: user_info.picture,
                email_verified: user_info.verified_email,
            })
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google user info error: {}", error_text);
            Err(oauth_error(OAuthErrorKind::InvalidResponse, &error_text))
        }
    }

    fn uses_rotating_refresh_tokens(&self) -> bool {
        false // Google does not rotate refresh tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::Provider as OAuthProvider;

    fn create_test_provider() -> Provider {
        Provider::new(
            "test_client_id".to_string(),
            "test_client_secret".to_string(),
            "https://example.com/callback".to_string(),
        )
    }

    #[test]
    fn test_provider_kind() {
        let provider = create_test_provider();
        assert_eq!(provider.provider(), ProviderKind::Google);
    }

    #[test]
    fn test_authorization_url_without_pkce() {
        let provider = create_test_provider();
        let auth_request = provider.authorization_url("test_state_123", None);

        assert!(auth_request.url.contains("client_id=test_client_id"));
        assert!(auth_request
            .url
            .contains("redirect_uri=https%3A%2F%2Fexample.com%2Fcallback"));
        assert!(auth_request.url.contains("response_type=code"));
        assert!(auth_request.url.contains("access_type=offline"));
        assert!(auth_request.url.contains("prompt=consent"));
        assert!(auth_request.url.contains("state=test_state_123"));
        assert!(auth_request.url.contains("scope="));
        assert_eq!(auth_request.state, "test_state_123");
        assert_eq!(auth_request.pkce_verifier, None);
    }

    #[test]
    fn test_authorization_url_with_pkce() {
        let provider = create_test_provider();
        let auth_request = provider.authorization_url("test_state_456", Some("test_challenge"));

        assert!(auth_request.url.contains("code_challenge=test_challenge"));
        assert!(auth_request.url.contains("code_challenge_method=S256"));
        assert_eq!(auth_request.state, "test_state_456");
    }

    #[test]
    fn test_authorization_url_includes_required_scopes() {
        let provider = create_test_provider();
        let auth_request = provider.authorization_url("state", None);

        // Check that the URL includes the required scopes (URL encoded)
        assert!(auth_request.url.contains("openid"));
        assert!(auth_request.url.contains("email"));
        assert!(auth_request.url.contains("profile"));
        assert!(auth_request.url.contains("meetings.space.created"));
    }

    #[test]
    fn test_does_not_use_rotating_refresh_tokens() {
        let provider = create_test_provider();
        assert!(!provider.uses_rotating_refresh_tokens());
    }
}
