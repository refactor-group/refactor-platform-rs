//! Recall.ai API client for recording bot management and async transcription.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use log::*;
use serde::{Deserialize, Serialize};

/// Recall.ai provider client. Constructed from system-level config at each call site.
pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    webhook_url: String,
}

#[derive(Debug, Serialize)]
struct CreateBotRequest {
    meeting_url: String,
    bot_name: String,
    webhook_url: String,
    metadata: BotMetadata,
}

#[derive(Debug, Serialize)]
struct BotMetadata {
    coaching_session_id: String,
}

/// Response from the Recall.ai create bot endpoint.
#[derive(Debug, Deserialize)]
pub struct BotResponse {
    pub id: String,
}

#[derive(Debug, Serialize)]
struct CreateTranscriptRequest {
    assemblyai: AssemblyAiConfig,
    diarization: DiarizationConfig,
}

#[derive(Debug, Serialize)]
struct AssemblyAiConfig {
    language_detection: bool,
    sentiment_analysis: bool,
    speaker_labels: bool,
}

#[derive(Debug, Serialize)]
struct DiarizationConfig {
    use_separate_streams_when_available: bool,
}

/// Response from the Recall.ai async transcript endpoints.
#[derive(Debug, Deserialize)]
pub struct TranscriptMetadata {
    pub id: String,
    pub status: Option<String>,
    pub download_url: Option<String>,
    pub language_code: Option<String>,
    pub speaker_count: Option<i64>,
    pub confidence: Option<f64>,
    pub duration: Option<f64>,
    pub word_count: Option<i64>,
}

/// Transcript JSON downloaded from the pre-signed `download_url`.
#[derive(Debug, Deserialize)]
pub struct TranscriptData {
    pub utterances: Option<Vec<Utterance>>,
    pub confidence: Option<f64>,
    pub audio_duration: Option<f64>,
    pub language_code: Option<String>,
    pub words: Option<Vec<Word>>,
}

/// A speaker-diarized utterance in the transcript.
#[derive(Debug, Deserialize)]
pub struct Utterance {
    pub speaker: String,
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub confidence: Option<f64>,
    pub sentiment: Option<String>,
    pub words: Option<Vec<Word>>,
}

/// A single word in a transcript utterance.
#[derive(Debug, Deserialize)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub confidence: Option<f64>,
}

impl Provider {
    /// Construct a provider from the system API key, region, and webhook callback URL.
    pub fn new(api_key: &str, region: &str, webhook_url: &str) -> Result<Self, Error> {
        let mut headers = reqwest::header::HeaderMap::new();

        let auth_value = format!("Token {}", api_key);
        let mut header_value =
            reqwest::header::HeaderValue::from_str(&auth_value).map_err(|e| {
                warn!("Failed to create Recall.ai auth header: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::Internal(InternalErrorKind::Other(
                        "Invalid Recall.ai API key format".to_string(),
                    )),
                }
            })?;
        header_value.set_sensitive(true);
        headers.insert(reqwest::header::AUTHORIZATION, header_value);

        let client = reqwest::Client::builder()
            .use_rustls_tls()
            .default_headers(headers)
            .build()?;

        let base_url = format!("https://{}.recall.ai/api/v1", region);

        Ok(Self {
            client,
            base_url,
            webhook_url: webhook_url.to_string(),
        })
    }

    /// Creates a Recall.ai recording bot for the given meeting URL.
    ///
    /// The `coaching_session_id` is embedded in bot metadata so that webhook handlers
    /// can route events back to the correct session without a database lookup.
    pub async fn create_bot(
        &self,
        coaching_session_id: &str,
        meeting_url: &str,
        bot_name: &str,
    ) -> Result<BotResponse, Error> {
        let url = format!("{}/bot", self.base_url);

        let request = CreateBotRequest {
            meeting_url: meeting_url.to_string(),
            bot_name: bot_name.to_string(),
            webhook_url: self.webhook_url.clone(),
            metadata: BotMetadata {
                coaching_session_id: coaching_session_id.to_string(),
            },
        };

        debug!("Creating Recall.ai bot for session {}", coaching_session_id);

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
            let bot: BotResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai bot response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Recall.ai bot API".to_string(),
                    )),
                }
            })?;
            info!("Created Recall.ai bot: {}", bot.id);
            Ok(bot)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai create bot error ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Removes a Recall.ai recording bot (stop and delete).
    pub async fn delete_bot(&self, bot_id: &str) -> Result<(), Error> {
        let url = format!("{}/bot/{}", self.base_url, bot_id);

        debug!("Deleting Recall.ai bot {}", bot_id);

        let response = self.client.delete(&url).send().await.map_err(|e| {
            warn!("Failed to delete Recall.ai bot {}: {:?}", bot_id, e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND {
            info!("Deleted Recall.ai bot {}", bot_id);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai delete bot error ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Triggers async transcription for the recording with the given bot ID.
    ///
    /// Uses AssemblyAI via Recall.ai with speaker diarization and sentiment analysis.
    /// Returns the Recall.ai transcript ID; completion is signaled via `transcript.done` webhook.
    pub async fn create_async_transcript(&self, bot_id: &str) -> Result<String, Error> {
        let url = format!("{}/recordings/{}/async-transcripts", self.base_url, bot_id);

        let request = CreateTranscriptRequest {
            assemblyai: AssemblyAiConfig {
                language_detection: true,
                sentiment_analysis: true,
                speaker_labels: true,
            },
            diarization: DiarizationConfig {
                use_separate_streams_when_available: true,
            },
        };

        debug!("Creating async transcript for bot {}", bot_id);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to create Recall.ai async transcript: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let transcript: TranscriptMetadata = response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai transcript response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from Recall.ai transcript API".to_string(),
                    )),
                }
            })?;
            info!("Created Recall.ai async transcript: {}", transcript.id);
            Ok(transcript.id)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!(
                "Recall.ai async transcript error ({}): {}",
                status, error_text
            );
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Retrieves transcript metadata (including `download_url`) after `transcript.done`.
    pub async fn get_async_transcript(
        &self,
        bot_id: &str,
        transcript_id: &str,
    ) -> Result<TranscriptMetadata, Error> {
        let url = format!(
            "{}/recordings/{}/async-transcripts/{}",
            self.base_url, bot_id, transcript_id
        );

        debug!(
            "Retrieving async transcript {} for bot {}",
            transcript_id, bot_id
        );

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to get Recall.ai async transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            let transcript: TranscriptMetadata = response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai transcript metadata: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid transcript metadata from Recall.ai".to_string(),
                    )),
                }
            })?;
            Ok(transcript)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!(
                "Recall.ai get transcript error ({}): {}",
                status, error_text
            );
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Downloads the transcript JSON from a pre-signed URL (no auth header required).
    pub async fn download_transcript(&self, download_url: &str) -> Result<TranscriptData, Error> {
        debug!("Downloading transcript from pre-signed URL");

        let response = reqwest::get(download_url).await.map_err(|e| {
            warn!("Failed to download transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            let data: TranscriptData = response.json().await.map_err(|e| {
                warn!("Failed to parse transcript data: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid transcript JSON from download URL".to_string(),
                    )),
                }
            })?;
            Ok(data)
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Transcript download error ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}
