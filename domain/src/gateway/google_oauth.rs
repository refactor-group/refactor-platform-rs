//! Google OAuth and Meet API client.
//!
//! This module provides an HTTP client for interacting with Google OAuth
//! and the Google Meet API to create meeting spaces.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde::{Deserialize, Serialize};

/// OAuth token response from Google
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in: i64,
    pub token_type: String,
    #[serde(default)]
    pub scope: String,
}

/// User info from Google
#[derive(Debug, Deserialize)]
pub struct GoogleUserInfo {
    pub id: String,
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub picture: Option<String>,
}

/// Request to exchange authorization code for tokens
#[derive(Debug, Serialize)]
struct TokenExchangeRequest {
    code: String,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    grant_type: String,
}

/// Request to refresh access token
#[derive(Debug, Serialize)]
struct TokenRefreshRequest {
    refresh_token: String,
    client_id: String,
    client_secret: String,
    grant_type: String,
}

/// Google Meet space configuration
#[derive(Debug, Serialize)]
pub struct SpaceConfig {
    #[serde(rename = "accessType")]
    pub access_type: String,
}

/// Request to create a Google Meet space
#[derive(Debug, Serialize)]
pub struct CreateSpaceRequest {
    pub config: SpaceConfig,
}

/// Response from creating a Google Meet space
#[derive(Debug, Deserialize)]
pub struct SpaceResponse {
    pub name: String,
    #[serde(rename = "meetingUri")]
    pub meeting_uri: String,
    #[serde(rename = "meetingCode")]
    pub meeting_code: String,
}

/// Configuration for Google OAuth URLs
#[derive(Debug, Clone)]
pub struct GoogleOAuthUrls {
    pub auth_url: String,
    pub token_url: String,
    pub userinfo_url: String,
}

/// Google OAuth client for handling authentication and Meet API
pub struct GoogleOAuthClient {
    client: reqwest::Client,
    client_id: String,
    client_secret: String,
    redirect_uri: String,
    urls: GoogleOAuthUrls,
}

impl GoogleOAuthClient {
    /// Create a new Google OAuth client with configurable URLs
    pub fn new(
        client_id: &str,
        client_secret: &str,
        redirect_uri: &str,
        urls: GoogleOAuthUrls,
    ) -> Result<Self, Error> {
        let client = reqwest::Client::builder().use_rustls_tls().build()?;

        Ok(Self {
            client,
            client_id: client_id.to_string(),
            client_secret: client_secret.to_string(),
            redirect_uri: redirect_uri.to_string(),
            urls,
        })
    }

    /// Generate the OAuth authorization URL for user consent
    pub fn get_authorization_url(&self, state: &str) -> String {
        let scopes = [
            "openid",
            "email",
            "profile",
            "https://www.googleapis.com/auth/meetings.space.created",
        ]
        .join(" ");

        format!(
            "{}?\
            client_id={}&\
            redirect_uri={}&\
            response_type=code&\
            scope={}&\
            access_type=offline&\
            prompt=consent&\
            state={}",
            self.urls.auth_url,
            urlencoding::encode(&self.client_id),
            urlencoding::encode(&self.redirect_uri),
            urlencoding::encode(&scopes),
            urlencoding::encode(state)
        )
    }

    /// Exchange authorization code for access and refresh tokens
    pub async fn exchange_code(&self, code: &str) -> Result<TokenResponse, Error> {
        let request = TokenExchangeRequest {
            code: code.to_string(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            redirect_uri: self.redirect_uri.clone(),
            grant_type: "authorization_code".to_string(),
        };

        debug!("Exchanging Google OAuth code for tokens");

        let response = self
            .client
            .post(&self.urls.token_url)
            .form(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to exchange Google OAuth code: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let tokens: TokenResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Google token response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Google OAuth".to_string(),
                    )),
                }
            })?;
            info!("Successfully exchanged Google OAuth code for tokens");
            Ok(tokens)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google OAuth error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Refresh an expired access token using the refresh token
    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponse, Error> {
        let request = TokenRefreshRequest {
            refresh_token: refresh_token.to_string(),
            client_id: self.client_id.clone(),
            client_secret: self.client_secret.clone(),
            grant_type: "refresh_token".to_string(),
        };

        debug!("Refreshing Google access token");

        let response = self
            .client
            .post(&self.urls.token_url)
            .form(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to refresh Google token: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let tokens: TokenResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Google token refresh response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Google OAuth".to_string(),
                    )),
                }
            })?;
            info!("Successfully refreshed Google access token");
            Ok(tokens)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google token refresh error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Get user info using the access token
    pub async fn get_user_info(&self, access_token: &str) -> Result<GoogleUserInfo, Error> {
        let response = self
            .client
            .get(&self.urls.userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to get Google user info: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let user_info: GoogleUserInfo = response.json().await.map_err(|e| {
                warn!("Failed to parse Google user info: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Google".to_string(),
                    )),
                }
            })?;
            Ok(user_info)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google user info error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Verify if an access token is still valid
    pub async fn verify_token(&self, access_token: &str) -> Result<bool, Error> {
        let response = self
            .client
            .get(&self.urls.userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to verify Google token: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        Ok(response.status().is_success())
    }
}

/// Google Meet API client for creating meeting spaces
pub struct GoogleMeetClient {
    client: reqwest::Client,
    base_url: String,
}

impl GoogleMeetClient {
    /// Create a new Google Meet client with the given access token and base URL
    pub fn new(access_token: &str, base_url: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();

        let auth_value = format!("Bearer {}", access_token);
        let mut header_value =
            reqwest::header::HeaderValue::from_str(&auth_value).map_err(|e| {
                warn!("Failed to create auth header: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "Invalid access token format".to_string(),
                    )),
                }
            })?;
        header_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, header_value);

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
        })
    }

    /// Create a new Google Meet space
    pub async fn create_space(&self) -> Result<SpaceResponse, Error> {
        let url = format!("{}/spaces", self.base_url);

        let request = CreateSpaceRequest {
            config: SpaceConfig {
                access_type: "OPEN".to_string(),
            },
        };

        debug!("Creating Google Meet space");

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to create Google Meet space: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let space: SpaceResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Google Meet response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Google Meet API".to_string(),
                    )),
                }
            })?;
            info!("Created Google Meet space: {}", space.meeting_code);
            Ok(space)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Google Meet API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}
