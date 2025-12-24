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

/// Meeting URL info returned by Recall.ai
/// Note: This is an object, not a plain string URL
#[derive(Debug, Deserialize)]
pub struct MeetingUrlInfo {
    /// The meeting ID extracted from the URL
    pub meeting_id: String,
    /// The meeting platform (e.g., "google_meet", "zoom", "teams")
    pub platform: String,
}

/// Response from creating a bot
/// Note: The Recall.ai API returns many fields - we only capture what we need
#[derive(Debug, Deserialize)]
pub struct CreateBotResponse {
    /// Bot ID (could be "id" or "bot_id" depending on endpoint)
    #[serde(alias = "bot_id")]
    pub id: String,
    /// Meeting URL info (object with meeting_id and platform)
    #[serde(default)]
    pub meeting_url: Option<MeetingUrlInfo>,
    /// Bot name
    #[serde(default)]
    pub bot_name: Option<String>,
    /// Status changes (empty array on creation)
    #[serde(default)]
    pub status_changes: Vec<StatusChange>,
}

/// Bot status change
#[derive(Debug, Deserialize)]
pub struct StatusChange {
    pub code: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub sub_code: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Bot status response
#[derive(Debug, Deserialize)]
pub struct BotStatusResponse {
    pub id: String,
    pub status_changes: Vec<StatusChange>,
    /// Recordings array containing media artifacts
    #[serde(default)]
    pub recordings: Vec<Recording>,
    #[serde(default)]
    pub meeting_metadata: Option<MeetingMetadata>,
}

impl BotStatusResponse {
    /// Extract the video download URL from the nested recordings structure
    pub fn video_url(&self) -> Option<String> {
        self.recordings
            .first()
            .and_then(|r| r.media_shortcuts.as_ref())
            .and_then(|ms| ms.video_mixed.as_ref())
            .and_then(|vm| vm.data.as_ref())
            .and_then(|d| d.download_url.clone())
    }

    /// Extract duration from the first recording
    pub fn duration_seconds(&self) -> Option<i32> {
        self.recordings
            .first()
            .and_then(|r| match (&r.started_at, &r.completed_at) {
                (Some(start), Some(end)) => {
                    if let (Ok(s), Ok(e)) = (
                        chrono::DateTime::parse_from_rfc3339(start),
                        chrono::DateTime::parse_from_rfc3339(end),
                    ) {
                        Some((e - s).num_seconds() as i32)
                    } else {
                        None
                    }
                }
                _ => None,
            })
    }
}

/// Recording object from Recall.ai
#[derive(Debug, Deserialize)]
pub struct Recording {
    pub id: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub status: Option<RecordingStatusInfo>,
    #[serde(default)]
    pub media_shortcuts: Option<MediaShortcuts>,
}

/// Recording status info
#[derive(Debug, Deserialize)]
pub struct RecordingStatusInfo {
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub sub_code: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Media shortcuts containing video/audio artifacts
#[derive(Debug, Deserialize)]
pub struct MediaShortcuts {
    #[serde(default)]
    pub video_mixed: Option<MediaArtifact>,
}

/// Individual media artifact
#[derive(Debug, Deserialize)]
pub struct MediaArtifact {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub recording_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub status: Option<RecordingStatusInfo>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub data: Option<MediaData>,
    #[serde(default)]
    pub format: Option<String>,
}

/// Media data containing the download URL
#[derive(Debug, Deserialize)]
pub struct MediaData {
    #[serde(default)]
    pub download_url: Option<String>,
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
            // Get raw text first for debugging
            let response_text = response.text().await.map_err(|e| {
                warn!("Failed to read Recall.ai response body: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

            debug!("Recall.ai raw response: {}", response_text);

            let bot: CreateBotResponse = serde_json::from_str(&response_text).map_err(|e| {
                let error_msg = format!("Invalid response from Recall.ai: {}", e);
                warn!(
                    "Failed to parse Recall.ai response: {:?}. Raw response: {}",
                    e, response_text
                );
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_msg)),
                }
            })?;
            info!("Created Recall.ai bot with ID: {}", bot.id);
            Ok(bot)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            error!("Recall.ai API ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(format!(
                    "Recall.ai API ({}): {}",
                    status, error_text
                ))),
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
            error!("Recall.ai API: {}", error_text);
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
            error!("Recall.ai failed to stop bot: {}", error_text);
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
                // Real-time transcript events (during recording)
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
