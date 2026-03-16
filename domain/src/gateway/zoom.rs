//! Zoom API client for creating meetings.
//!
//! This module provides an HTTP client for interacting with the Zoom API
//! to create meetings.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use chrono::NaiveDateTime;
use log::*;
use serde::{Deserialize, Serialize};

/// Zoom meeting configuration
#[derive(Debug, Serialize)]
pub struct MeetingConfig {
    pub join_before_host: bool,

    // Allow joining at any time
    pub jbh_time: u64,
}

/// Request to create a Zoom meeting
#[derive(Debug, Serialize)]
pub struct CreateMeetingRequest {
    pub config: MeetingConfig,

    /// Start time (GMT format: yyyy-MM-ddTHH:mm:ssZ or local time with timezone)
    pub start_time: String,

    /// Duration in minutes
    pub duration: i32,
}

/// Response from creating a Zoom meeting
#[derive(Debug, Deserialize)]
pub struct MeetingResponse {
    pub id: i64,
    pub join_url: String,
    pub start_url: String,
    pub topic: String,
}

/// Zoom API client
pub struct Client {
    client: reqwest::Client,
    base_url: String,
}

impl Client {
    /// Create a new Zoom client with the given access token and base URL
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

    /// Create a new Zoom meeting
    pub async fn create_meeting(
        &self,
        start_time: &NaiveDateTime,
        external_user_id: &str,
    ) -> Result<MeetingResponse, Error> {
        let url = format!("{}/users/{}/meetings", self.base_url, external_user_id);

        let request = CreateMeetingRequest {
            start_time: start_time.to_string(),
            duration: 60,
            config: MeetingConfig {
                join_before_host: true,
                jbh_time: 0,
            },
        };

        debug!("Creating Zoom meeting");

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to create Zoom meeting: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let meeting: MeetingResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Zoom response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Zoom API".to_string(),
                    )),
                }
            })?;
            info!("Created Zoom meeting: {}", meeting.join_url);
            Ok(meeting)
        } else if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            warn!("Zoom API returned 401 Unauthorized: access token expired or revoked");
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::OauthTokenRevoked(
                    "zoom".to_string(),
                )),
            })
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Zoom API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}
