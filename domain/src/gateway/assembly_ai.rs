//! AssemblyAI API client for transcription services.
//!
//! This module provides an HTTP client for interacting with the AssemblyAI API
//! to transcribe meeting recordings with speaker diarization and AI features.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde::{Deserialize, Serialize};

/// Request to create a new transcription
#[derive(Debug, Serialize)]
pub struct CreateTranscriptRequest {
    pub audio_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_auth_header_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_auth_header_value: Option<String>,
    pub speaker_labels: bool,
    pub sentiment_analysis: bool,
    pub auto_chapters: bool,
    pub entity_detection: bool,
}

/// Response from creating a transcript
#[derive(Debug, Deserialize)]
pub struct TranscriptResponse {
    pub id: String,
    pub status: TranscriptStatus,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub words: Option<Vec<Word>>,
    #[serde(default)]
    pub utterances: Option<Vec<Utterance>>,
    #[serde(default)]
    pub chapters: Option<Vec<Chapter>>,
    #[serde(default)]
    pub sentiment_analysis_results: Option<Vec<SentimentResult>>,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub audio_duration: Option<i64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// Transcript processing status
#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptStatus {
    Queued,
    Processing,
    Completed,
    Error,
}

/// Word with timing information
#[derive(Debug, Deserialize, Clone)]
pub struct Word {
    pub text: String,
    pub start: i64,
    pub end: i64,
    pub confidence: f64,
    #[serde(default)]
    pub speaker: Option<String>,
}

/// Utterance (speaker segment) with timing
#[derive(Debug, Deserialize, Clone)]
pub struct Utterance {
    pub text: String,
    pub start: i64,
    pub end: i64,
    pub confidence: f64,
    pub speaker: String,
    #[serde(default)]
    pub words: Option<Vec<Word>>,
}

/// Auto-generated chapter
#[derive(Debug, Deserialize, Clone)]
pub struct Chapter {
    pub summary: String,
    pub headline: String,
    pub start: i64,
    pub end: i64,
    pub gist: String,
}

/// Sentiment analysis result
#[derive(Debug, Deserialize, Clone)]
pub struct SentimentResult {
    pub text: String,
    pub start: i64,
    pub end: i64,
    pub sentiment: Sentiment,
    pub confidence: f64,
    #[serde(default)]
    pub speaker: Option<String>,
}

/// Sentiment classification
#[derive(Debug, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
}

/// AssemblyAI API client
pub struct AssemblyAiClient {
    client: reqwest::Client,
    base_url: String,
}

impl AssemblyAiClient {
    /// Create a new AssemblyAI client with the given API key and base URL
    pub fn new(api_key: &str, base_url: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();

        let mut header_value = reqwest::header::HeaderValue::from_str(api_key).map_err(|e| {
            warn!("Failed to create auth header: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                    "Invalid API key format".to_string(),
                )),
            }
        })?;
        header_value.set_sensitive(true);
        headers.insert("authorization", header_value);

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .default_headers(headers)
            .build()?;

        Ok(Self {
            client,
            base_url: base_url.to_string(),
        })
    }

    /// Verify the API key is valid by making a test request
    pub async fn verify_api_key(&self) -> Result<bool, Error> {
        let url = format!("{}/transcript", self.base_url);

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to verify AssemblyAI API key: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        // 200 means valid key (returns list of transcripts)
        // 401 means invalid key
        Ok(response.status().is_success())
    }

    /// Create a new transcription request
    pub async fn create_transcript(
        &self,
        request: CreateTranscriptRequest,
    ) -> Result<TranscriptResponse, Error> {
        let url = format!("{}/transcript", self.base_url);

        debug!(
            "Creating AssemblyAI transcript for audio: {}",
            request.audio_url
        );

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to create AssemblyAI transcript: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let transcript: TranscriptResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse AssemblyAI response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from AssemblyAI".to_string(),
                    )),
                }
            })?;
            info!("Created AssemblyAI transcript with ID: {}", transcript.id);
            Ok(transcript)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("AssemblyAI API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Get the status of a transcript
    pub async fn get_transcript(&self, transcript_id: &str) -> Result<TranscriptResponse, Error> {
        let url = format!("{}/transcript/{}", self.base_url, transcript_id);

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to get AssemblyAI transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            let transcript: TranscriptResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse AssemblyAI response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from AssemblyAI".to_string(),
                    )),
                }
            })?;
            Ok(transcript)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("AssemblyAI API error: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Delete a transcript
    pub async fn delete_transcript(&self, transcript_id: &str) -> Result<(), Error> {
        let url = format!("{}/transcript/{}", self.base_url, transcript_id);

        let response = self.client.delete(&url).send().await.map_err(|e| {
            warn!("Failed to delete AssemblyAI transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            info!("Deleted AssemblyAI transcript: {}", transcript_id);
            Ok(())
        } else {
            let error_text = response.text().await.unwrap_or_default();
            warn!("Failed to delete AssemblyAI transcript: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}

/// Helper to create a standard transcript request with webhook configuration
pub fn create_standard_transcript_request(
    audio_url: String,
    webhook_url: Option<String>,
    webhook_secret: Option<String>,
) -> CreateTranscriptRequest {
    CreateTranscriptRequest {
        audio_url,
        webhook_url: webhook_url.clone(),
        webhook_auth_header_name: webhook_url.as_ref().map(|_| "X-Webhook-Secret".to_string()),
        webhook_auth_header_value: webhook_secret,
        speaker_labels: true,
        sentiment_analysis: true,
        auto_chapters: true,
        entity_detection: true,
    }
}

/// Extract action items from transcript chapters and sentiment
/// This is a simple extraction - in production, you might use a more sophisticated approach
pub fn extract_action_items(transcript: &TranscriptResponse) -> Vec<String> {
    let mut actions = Vec::new();

    // Look for action-related phrases in the transcript text
    if let Some(text) = &transcript.text {
        let action_keywords = [
            "i will",
            "i'll",
            "we will",
            "we'll",
            "going to",
            "need to",
            "should",
            "must",
            "have to",
            "by friday",
            "by monday",
            "next week",
            "tomorrow",
        ];

        for sentence in text.split(['.', '!', '?']) {
            let lower = sentence.to_lowercase();
            if action_keywords.iter().any(|kw| lower.contains(kw)) {
                actions.push(sentence.trim().to_string());
            }
        }
    }

    actions
}
