//! Recall.ai API client for meeting recording bot management.
//!
//! This module provides an HTTP client for interacting with the Recall.ai API
//! to manage meeting recording bots for Google Meet sessions.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Recall.ai API regions
#[derive(Debug, Clone, Default)]
pub enum RecallRegion {
    #[default]
    UsWest2,
    UsEast1,
    EuWest1,
}

impl RecallRegion {
    /// Returns the region code (e.g., "us-west-2")
    pub fn as_str(&self) -> &'static str {
        match self {
            RecallRegion::UsWest2 => "us-west-2",
            RecallRegion::UsEast1 => "us-east-1",
            RecallRegion::EuWest1 => "eu-west-1",
        }
    }

    /// Constructs the full base URL using the given base domain
    pub fn base_url(&self, base_domain: &str) -> String {
        format!("https://{}.{}", self.as_str(), base_domain)
    }
}

impl FromStr for RecallRegion {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.to_lowercase().as_str() {
            "us-east-1" => RecallRegion::UsEast1,
            "eu-west-1" => RecallRegion::EuWest1,
            _ => RecallRegion::UsWest2,
        })
    }
}

/// Request to create a new recording bot
#[derive(Debug, Serialize)]
pub struct CreateBotRequest {
    pub meeting_url: String,
    pub bot_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording_config: Option<RecordingConfig>,
}

/// Recording configuration for the bot
#[derive(Debug, Serialize)]
pub struct RecordingConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<TranscriptConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub realtime_endpoints: Option<Vec<RealtimeEndpoint>>,
}

/// Transcript configuration
#[derive(Debug, Serialize)]
pub struct TranscriptConfig {
    pub provider: TranscriptProvider,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diarization: Option<DiarizationConfig>,
}

/// Transcript provider configuration
#[derive(Debug, Serialize)]
pub struct TranscriptProvider {
    pub recallai_streaming: StreamingMode,
}

/// Streaming mode for transcription
#[derive(Debug, Serialize)]
pub struct StreamingMode {
    pub mode: String,
}

/// Diarization (speaker identification) configuration
#[derive(Debug, Serialize)]
pub struct DiarizationConfig {
    pub use_separate_streams_when_available: bool,
}

/// Realtime webhook endpoint configuration
#[derive(Debug, Serialize)]
pub struct RealtimeEndpoint {
    #[serde(rename = "type")]
    pub endpoint_type: String,
    pub url: String,
    pub events: Vec<String>,
}

/// Response from creating a bot
#[derive(Debug, Deserialize)]
pub struct CreateBotResponse {
    pub id: String,
    pub meeting_url: String,
    pub status_changes: Vec<StatusChange>,
}

/// Bot status change
#[derive(Debug, Deserialize)]
pub struct StatusChange {
    pub code: String,
    pub created_at: String,
}

/// Bot status response
#[derive(Debug, Deserialize)]
pub struct BotStatusResponse {
    pub id: String,
    pub status_changes: Vec<StatusChange>,
    #[serde(default)]
    pub video_url: Option<String>,
    #[serde(default)]
    pub meeting_metadata: Option<MeetingMetadata>,
}

/// Meeting metadata from the bot
#[derive(Debug, Deserialize)]
pub struct MeetingMetadata {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub duration: Option<i64>,
}

/// Recall.ai API client
pub struct RecallAiClient {
    client: reqwest::Client,
    base_url: String,
}

impl RecallAiClient {
    /// Create a new Recall.ai client with the given API key, region, and base domain
    pub fn new(api_key: &str, region: RecallRegion, base_domain: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();

        let auth_value = format!("Token {}", api_key);
        let mut header_value =
            reqwest::header::HeaderValue::from_str(&auth_value).map_err(|e| {
                warn!("Failed to create auth header: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "Invalid API key format".to_string(),
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
            base_url: region.base_url(base_domain),
        })
    }

    /// Verify the API key is valid by making a test request
    pub async fn verify_api_key(&self) -> Result<bool, Error> {
        let url = format!("{}/api/v1/bot/", self.base_url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to verify Recall.ai API key: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        // 200 or 401 both indicate the API is reachable
        // 401 means invalid key, 200 means valid key
        Ok(response.status().is_success())
    }

    /// Create a new recording bot for a meeting
    pub async fn create_bot(&self, request: CreateBotRequest) -> Result<CreateBotResponse, Error> {
        let url = format!("{}/api/v1/bot/", self.base_url);

        debug!(
            "Creating Recall.ai bot for meeting: {}",
            request.meeting_url
        );

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to create Recall.ai bot: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let bot: CreateBotResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Recall.ai".to_string(),
                    )),
                }
            })?;
            info!("Created Recall.ai bot with ID: {}", bot.id);
            Ok(bot)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Get the status of a bot
    pub async fn get_bot_status(&self, bot_id: &str) -> Result<BotStatusResponse, Error> {
        let url = format!("{}/api/v1/bot/{}/", self.base_url, bot_id);

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to get Recall.ai bot status: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            let status: BotStatusResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai status response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Recall.ai".to_string(),
                    )),
                }
            })?;
            Ok(status)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Stop a recording bot
    pub async fn stop_bot(&self, bot_id: &str) -> Result<(), Error> {
        let url = format!("{}/api/v1/bot/{}/leave_call/", self.base_url, bot_id);

        debug!("Stopping Recall.ai bot: {}", bot_id);

        let response = self.client.post(&url).send().await.map_err(|e| {
            warn!("Failed to stop Recall.ai bot: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            info!("Stopped Recall.ai bot: {}", bot_id);
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to stop Recall.ai bot: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}

/// Helper to create a standard bot request with webhook configuration
pub fn create_standard_bot_request(
    meeting_url: String,
    bot_name: String,
    webhook_url: Option<String>,
) -> CreateBotRequest {
    let recording_config = webhook_url.map(|url| RecordingConfig {
        transcript: Some(TranscriptConfig {
            provider: TranscriptProvider {
                recallai_streaming: StreamingMode {
                    mode: "prioritize_accuracy".to_string(),
                },
            },
            diarization: Some(DiarizationConfig {
                use_separate_streams_when_available: true,
            }),
        }),
        realtime_endpoints: Some(vec![RealtimeEndpoint {
            endpoint_type: "webhook".to_string(),
            url,
            events: vec![
                "transcript.data".to_string(),
                "transcript.partial_data".to_string(),
            ],
        }]),
    });

    CreateBotRequest {
        meeting_url,
        bot_name,
        recording_config,
    }
}
