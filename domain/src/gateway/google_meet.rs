//! Google Meet API client for creating meeting spaces.
//!
//! This module provides an HTTP client for interacting with the Google Meet API
//! to create meeting spaces.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde::{Deserialize, Serialize};

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

/// Google Meet API client
pub struct Client {
    client: reqwest::Client,
    base_url: String,
}

impl Client {
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
