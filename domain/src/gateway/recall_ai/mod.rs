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
    provider: TranscriptProvider,
    diarization: DiarizationConfig,
}

#[derive(Debug, Serialize)]
struct TranscriptProvider {
    assembly_ai_async: AssemblyAiConfig,
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

#[derive(Debug, Deserialize)]
struct TranscriptLinks {
    download_url: Option<String>,
}

/// Response from the Recall.ai async transcript endpoints.
#[derive(Debug, Deserialize)]
pub struct TranscriptMetadata {
    pub id: String,
    data: Option<TranscriptLinks>,
}

impl TranscriptMetadata {
    pub fn download_url(&self) -> Option<&str> {
        self.data.as_ref()?.download_url.as_deref()
    }
}

/// One entry in the transcript download array — all words spoken by one participant.
#[derive(Debug, Deserialize)]
pub struct ParticipantEntry {
    pub participant: Participant,
    pub language_code: Option<String>,
    pub words: Vec<TranscriptWord>,
}

#[derive(Debug, Deserialize)]
pub struct Participant {
    pub id: Option<i64>,
    pub name: Option<String>,
    pub is_host: Option<bool>,
    pub platform: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    pub start_timestamp: Timestamp,
    pub end_timestamp: Timestamp,
}

/// Timestamp returned by Recall.ai. For async transcription `absolute` is always null;
/// use `relative` (seconds from recording start).
#[derive(Debug, Deserialize)]
pub struct Timestamp {
    pub absolute: Option<String>,
    pub relative: Option<f64>,
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

    /// Triggers async transcription for the given Recall.ai recording.
    ///
    /// `recall_recording_id` is Recall's recording UUID from the `recording.done` webhook
    /// (`data.recording.id`) — distinct from the bot ID. Uses AssemblyAI with speaker
    /// diarization and sentiment analysis. Completion is signaled via `transcript.done` webhook.
    pub async fn create_async_transcript(
        &self,
        recall_recording_id: &str,
    ) -> Result<String, Error> {
        let url = format!(
            "{}/recording/{}/create_transcript/",
            self.base_url, recall_recording_id
        );

        let request = CreateTranscriptRequest {
            provider: TranscriptProvider {
                assembly_ai_async: AssemblyAiConfig {
                    language_detection: true,
                    sentiment_analysis: true,
                    speaker_labels: true,
                },
            },
            diarization: DiarizationConfig {
                use_separate_streams_when_available: true,
            },
        };

        debug!(
            "Creating async transcript for recording {}",
            recall_recording_id
        );

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
    ///
    /// `recall_recording_id` is Recall's recording UUID (`data.recording.id` from webhooks).
    pub async fn get_async_transcript(
        &self,
        recall_recording_id: &str,
        transcript_id: &str,
    ) -> Result<TranscriptMetadata, Error> {
        let url = format!(
            "{}/recording/{}/transcript/{}/",
            self.base_url, recall_recording_id, transcript_id
        );

        debug!(
            "Retrieving async transcript {} for recording {}",
            transcript_id, recall_recording_id
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
    ///
    /// Returns a participant-entry array as Recall.ai delivers it. Callers are responsible
    /// for coalescing words into speaker turns.
    pub async fn download_transcript(
        &self,
        download_url: &str,
    ) -> Result<Vec<ParticipantEntry>, Error> {
        debug!("Downloading transcript from pre-signed URL");

        let response = reqwest::get(download_url).await.map_err(|e| {
            warn!("Failed to download transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            let data: Vec<ParticipantEntry> = response.json().await.map_err(|e| {
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
