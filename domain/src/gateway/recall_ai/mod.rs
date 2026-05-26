//! Recall.ai API client for recording bot management and async transcription.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::*;
use meeting_ai::traits::{recording_bot, transcription as transcription_trait};
use meeting_ai::types::{recording as recording_types, transcription as transcription_types};
use serde::{Deserialize, Serialize};

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};

/// Recall.ai provider client. Built once at startup and shared via `AppState`.
#[derive(Clone)]
pub struct Provider {
    client: reqwest::Client,
    /// Header-less rustls client for pre-signed transcript downloads.
    /// Pre-signed URLs reject extra Authorization headers, so this cannot reuse `client`.
    download_client: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Serialize)]
struct CreateBotRequest {
    meeting_url: String,
    bot_name: String,
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
    speech_models: Vec<&'static str>,
    language_detection: bool,
    sentiment_analysis: bool,
}

#[derive(Debug, Serialize)]
struct DiarizationConfig {
    use_separate_streams_when_available: bool,
}

#[derive(Debug, Deserialize)]
struct TranscriptLinks {
    download_url: Option<String>,
}

/// Recall.ai returns `status` as an object: `{"code": "processing", "message": null}`.
#[derive(Debug, Deserialize)]
struct TranscriptStatusField {
    code: Option<String>,
}

/// Response from the Recall.ai async transcript endpoints.
#[derive(Debug, Deserialize)]
pub struct TranscriptMetadata {
    pub id: String,
    status: Option<TranscriptStatusField>,
    data: Option<TranscriptLinks>,
}

impl TranscriptMetadata {
    pub fn download_url(&self) -> Option<&str> {
        self.data.as_ref()?.download_url.as_deref()
    }

    pub fn status_str(&self) -> Option<&str> {
        self.status.as_ref()?.code.as_deref()
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
    pub extra_data: Option<serde_json::Value>,
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

/// Response from the Recall.ai GET /bot/{id}/ endpoint.
#[derive(Debug, Deserialize)]
struct BotDetailResponse {
    id: String,
    meeting_url: Option<String>,
    status_changes: Option<Vec<RecallBotStatusChange>>,
}

#[derive(Debug, Deserialize)]
struct RecallBotStatusChange {
    code: String,
    message: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

/// Response from the Recall.ai GET /bot/ endpoint.
#[derive(Debug, Deserialize)]
struct BotListResponse {
    #[serde(default)]
    results: Vec<BotDetailResponse>,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn to_meeting_ai_err(e: Error) -> meeting_ai::Error {
    match e.error_kind {
        DomainErrorKind::External(ExternalErrorKind::Network) => {
            meeting_ai::Error::Network("network error".to_string())
        }
        DomainErrorKind::External(ExternalErrorKind::Other(msg)) => {
            meeting_ai::Error::Provider(msg)
        }
        DomainErrorKind::Internal(InternalErrorKind::Config) => {
            meeting_ai::Error::Configuration("invalid configuration".to_string())
        }
        other => meeting_ai::Error::Provider(format!("{other:?}")),
    }
}

fn recall_status_to_recording_status(code: &str) -> recording_types::Status {
    match code {
        "joining_call" => recording_types::Status::Joining,
        "waiting_room" => recording_types::Status::WaitingRoom,
        "in_call_not_recording" => recording_types::Status::InMeeting,
        "in_call_recording" => recording_types::Status::Recording,
        "recording_done" | "call_ended" => recording_types::Status::Processing,
        "done" => recording_types::Status::Completed,
        "fatal_error" => recording_types::Status::Failed,
        _ => recording_types::Status::Pending,
    }
}

fn recall_transcript_status(status_str: Option<&str>) -> transcription_types::Status {
    match status_str {
        Some("processing") => transcription_types::Status::Processing,
        Some("complete") | Some("completed") => transcription_types::Status::Completed,
        Some("failed") => transcription_types::Status::Failed,
        _ => transcription_types::Status::Queued,
    }
}

/// Maximum silence gap (seconds) between same-speaker words before starting a new segment.
const SEGMENT_GAP_SECS: f64 = 1.5;

/// Coalesces Recall.ai participant word entries into meeting-ai `Segment` objects.
///
/// Words are sorted chronologically, then grouped by speaker with a gap threshold.
fn coalesce_entries(entries: Vec<ParticipantEntry>) -> Vec<transcription_types::Segment> {
    struct WordEntry {
        speaker: String,
        text: String,
        start_s: f64,
        end_s: f64,
    }

    let mut all_words: Vec<WordEntry> = Vec::new();
    for entry in &entries {
        let speaker = entry
            .participant
            .name
            .as_deref()
            .filter(|n| !n.is_empty())
            .map(|n| n.to_string())
            .or_else(|| entry.participant.id.map(|id| id.to_string()))
            .unwrap_or_else(|| "Unknown".to_string());

        for word in &entry.words {
            let start_s = word.start_timestamp.relative.unwrap_or(0.0);
            let end_s = word.end_timestamp.relative.unwrap_or(start_s);
            all_words.push(WordEntry {
                speaker: speaker.clone(),
                text: word.text.clone(),
                start_s,
                end_s,
            });
        }
    }

    all_words.sort_by(|a, b| {
        a.start_s
            .partial_cmp(&b.start_s)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut segments: Vec<transcription_types::Segment> = Vec::new();
    if let Some(first) = all_words.first() {
        let mut seg_speaker = first.speaker.clone();
        let mut seg_words: Vec<String> = vec![first.text.clone()];
        let mut seg_start = first.start_s;
        let mut seg_end = first.end_s;

        for word in all_words.iter().skip(1) {
            let same_speaker = word.speaker == seg_speaker;
            let small_gap = word.start_s - seg_end < SEGMENT_GAP_SECS;

            if same_speaker && small_gap {
                seg_words.push(word.text.clone());
                seg_end = word.end_s;
            } else {
                segments.push(transcription_types::Segment {
                    text: seg_words.join(" "),
                    speaker: seg_speaker.clone(),
                    start_ms: (seg_start * 1000.0) as i64,
                    end_ms: (seg_end * 1000.0) as i64,
                    confidence: 0.0,
                    words: vec![],
                });
                seg_speaker = word.speaker.clone();
                seg_words = vec![word.text.clone()];
                seg_start = word.start_s;
                seg_end = word.end_s;
            }
        }
        segments.push(transcription_types::Segment {
            text: seg_words.join(" "),
            speaker: seg_speaker,
            start_ms: (seg_start * 1000.0) as i64,
            end_ms: (seg_end * 1000.0) as i64,
            confidence: 0.0,
            words: vec![],
        });
    }

    segments
}

fn bot_detail_to_info(detail: BotDetailResponse) -> recording_types::Info {
    let current_status = detail
        .status_changes
        .as_deref()
        .and_then(|changes| changes.last())
        .map(|c| recall_status_to_recording_status(&c.code))
        .unwrap_or(recording_types::Status::Pending);

    let error_message = detail
        .status_changes
        .as_deref()
        .and_then(|changes| changes.last())
        .filter(|c| c.code == "fatal_error")
        .and_then(|c| c.message.clone());

    let status_history = detail
        .status_changes
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| {
            Some(recording_types::StatusChange {
                status: recall_status_to_recording_status(&c.code),
                timestamp: c.created_at?,
                message: c.message,
            })
        })
        .collect();

    recording_types::Info {
        id: detail.id,
        meeting_url: detail.meeting_url.unwrap_or_default(),
        status: current_status,
        artifacts: None,
        error_message,
        status_history,
    }
}

// ---------------------------------------------------------------------------
// Provider: Recall.ai HTTP client
// ---------------------------------------------------------------------------

impl Provider {
    /// Construct a provider from the system API key and region.
    pub fn new(api_key: &str, region: &str) -> Result<Self, Error> {
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
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;

        let download_client = reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;

        let base_url = format!("https://{}.recall.ai/api/v1", region);

        Ok(Self {
            client,
            download_client,
            base_url,
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
        let url = format!("{}/bot/", self.base_url);

        let request = CreateBotRequest {
            meeting_url: meeting_url.to_string(),
            bot_name: bot_name.to_string(),
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

    /// Removes a Recall.ai bot from an active call.
    ///
    /// Uses `POST /bot/{id}/leave_call/` which is valid for bots currently in a meeting.
    pub async fn leave_call(&self, bot_id: &str) -> Result<(), Error> {
        let url = format!("{}/bot/{}/leave_call/", self.base_url, bot_id);

        debug!("Removing Recall.ai bot {} from call", bot_id);

        let response = self.client.post(&url).send().await.map_err(|e| {
            warn!(
                "Failed to remove Recall.ai bot {} from call: {:?}",
                bot_id, e
            );
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            info!("Removed Recall.ai bot {} from call", bot_id);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai leave_call error ({}): {}", status, error_text);
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
                    speech_models: vec!["universal-2"],
                    language_detection: true,
                    sentiment_analysis: false,
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
    pub async fn get_async_transcript(
        &self,
        transcript_id: &str,
    ) -> Result<TranscriptMetadata, Error> {
        let url = format!("{}/transcript/{}/", self.base_url, transcript_id);

        debug!("Retrieving async transcript {}", transcript_id);

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

        let response = self
            .download_client
            .get(download_url)
            .send()
            .await
            .map_err(|e| {
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

    async fn bot_detail(&self, bot_id: &str) -> Result<BotDetailResponse, Error> {
        let url = format!("{}/bot/{}/", self.base_url, bot_id);

        debug!("Retrieving Recall.ai bot detail {}", bot_id);

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to get Recall.ai bot detail: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai bot detail: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid bot detail response from Recall.ai".to_string(),
                    )),
                }
            })
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai bot detail error ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    async fn list_bots_raw(&self) -> Result<BotListResponse, Error> {
        let url = format!("{}/bot/", self.base_url);

        debug!("Listing Recall.ai bots");

        let response = self.client.get(&url).send().await.map_err(|e| {
            warn!("Failed to list Recall.ai bots: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            response.json().await.map_err(|e| {
                warn!("Failed to parse Recall.ai bot list: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid bot list response from Recall.ai".to_string(),
                    )),
                }
            })
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!("Recall.ai bot list error ({}): {}", status, error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    async fn delete_transcript_raw(&self, transcript_id: &str) -> Result<(), Error> {
        let url = format!("{}/transcript/{}/", self.base_url, transcript_id);

        debug!("Deleting Recall.ai transcript {}", transcript_id);

        let response = self.client.delete(&url).send().await.map_err(|e| {
            warn!("Failed to delete Recall.ai transcript: {:?}", e);
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
            }
        })?;

        if response.status().is_success() {
            info!("Deleted Recall.ai transcript {}", transcript_id);
            Ok(())
        } else {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            warn!(
                "Recall.ai delete transcript error ({}): {}",
                status, error_text
            );
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

#[async_trait]
impl recording_bot::Provider for Provider {
    async fn create_bot(
        &self,
        config: recording_types::Config,
    ) -> std::result::Result<recording_types::Info, meeting_ai::Error> {
        let coaching_session_id = config
            .provider_options
            .get("coaching_session_id")
            .ok_or_else(|| {
                meeting_ai::Error::Configuration(
                    "coaching_session_id required in provider_options".into(),
                )
            })?;

        let bot = self
            .create_bot(coaching_session_id, &config.meeting_url, &config.bot_name)
            .await
            .map_err(to_meeting_ai_err)?;

        Ok(recording_types::Info {
            id: bot.id,
            meeting_url: config.meeting_url,
            status: recording_types::Status::Pending,
            artifacts: None,
            error_message: None,
            status_history: vec![],
        })
    }

    async fn get_bot_status(
        &self,
        bot_id: &str,
    ) -> std::result::Result<recording_types::Info, meeting_ai::Error> {
        let detail = self.bot_detail(bot_id).await.map_err(to_meeting_ai_err)?;
        Ok(bot_detail_to_info(detail))
    }

    async fn stop_bot(&self, bot_id: &str) -> std::result::Result<(), meeting_ai::Error> {
        self.leave_call(bot_id).await.map_err(to_meeting_ai_err)
    }

    async fn list_bots(
        &self,
        _filters: Option<recording_types::Filters>,
    ) -> std::result::Result<Vec<recording_types::Info>, meeting_ai::Error> {
        let list = self.list_bots_raw().await.map_err(to_meeting_ai_err)?;
        Ok(list.results.into_iter().map(bot_detail_to_info).collect())
    }

    fn provider_id(&self) -> &str {
        "recall_ai"
    }
}

#[async_trait]
impl transcription_trait::Provider for Provider {
    async fn create_transcription(
        &self,
        config: transcription_types::Config,
    ) -> std::result::Result<transcription_types::Transcription, meeting_ai::Error> {
        let recall_recording_id = config
            .provider_options
            .get("recall_recording_id")
            .ok_or_else(|| {
                meeting_ai::Error::Configuration(
                    "recall_recording_id required in provider_options".into(),
                )
            })?;

        let transcript_id = self
            .create_async_transcript(recall_recording_id)
            .await
            .map_err(to_meeting_ai_err)?;

        Ok(transcription_types::Transcription {
            id: transcript_id,
            status: transcription_types::Status::Queued,
            text: None,
            words: vec![],
            segments: vec![],
            chapters: vec![],
            sentiment_analysis: vec![],
            confidence: None,
            duration_seconds: None,
            language_code: config.language_code,
            speaker_count: None,
            error_message: None,
        })
    }

    async fn get_transcription(
        &self,
        transcription_id: &str,
    ) -> std::result::Result<transcription_types::Transcription, meeting_ai::Error> {
        let metadata = self
            .get_async_transcript(transcription_id)
            .await
            .map_err(to_meeting_ai_err)?;

        let status = recall_transcript_status(metadata.status_str());

        let segments = if let Some(url) = metadata.download_url() {
            let entries = self
                .download_transcript(url)
                .await
                .map_err(to_meeting_ai_err)?;
            coalesce_entries(entries)
        } else {
            vec![]
        };

        Ok(transcription_types::Transcription {
            id: metadata.id,
            status,
            text: None,
            words: vec![],
            segments,
            chapters: vec![],
            sentiment_analysis: vec![],
            confidence: None,
            duration_seconds: None,
            language_code: None,
            speaker_count: None,
            error_message: None,
        })
    }

    async fn delete_transcription(
        &self,
        transcription_id: &str,
    ) -> std::result::Result<(), meeting_ai::Error> {
        self.delete_transcript_raw(transcription_id)
            .await
            .map_err(to_meeting_ai_err)
    }

    fn provider_id(&self) -> &str {
        "recall_ai"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meeting_ai::types::{recording as recording_types, transcription as transcription_types};

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn word(speaker: &str, text: &str, start: f64, end: f64) -> ParticipantEntry {
        ParticipantEntry {
            participant: Participant {
                id: None,
                name: Some(speaker.to_string()),
                is_host: None,
                platform: None,
                email: None,
                extra_data: None,
            },
            language_code: None,
            words: vec![TranscriptWord {
                text: text.to_string(),
                start_timestamp: Timestamp {
                    absolute: None,
                    relative: Some(start),
                },
                end_timestamp: Timestamp {
                    absolute: None,
                    relative: Some(end),
                },
            }],
        }
    }

    fn test_provider(base_url: &str) -> Provider {
        Provider {
            client: reqwest::Client::new(),
            download_client: reqwest::Client::new(),
            base_url: base_url.to_string(),
        }
    }

    // ── recall_status_to_recording_status ─────────────────────────────────────

    #[test]
    fn recall_status_maps_all_known_codes() {
        let cases = [
            ("joining_call", recording_types::Status::Joining),
            ("waiting_room", recording_types::Status::WaitingRoom),
            ("in_call_not_recording", recording_types::Status::InMeeting),
            ("in_call_recording", recording_types::Status::Recording),
            ("recording_done", recording_types::Status::Processing),
            ("call_ended", recording_types::Status::Processing),
            ("done", recording_types::Status::Completed),
            ("fatal_error", recording_types::Status::Failed),
            ("unknown_future_code", recording_types::Status::Pending),
        ];
        for (code, expected) in cases {
            assert_eq!(
                recall_status_to_recording_status(code),
                expected,
                "code={}",
                code
            );
        }
    }

    // ── recall_transcript_status ──────────────────────────────────────────────

    #[test]
    fn recall_transcript_status_maps_all_known_values() {
        assert_eq!(
            recall_transcript_status(Some("processing")),
            transcription_types::Status::Processing
        );
        assert_eq!(
            recall_transcript_status(Some("complete")),
            transcription_types::Status::Completed
        );
        assert_eq!(
            recall_transcript_status(Some("completed")),
            transcription_types::Status::Completed
        );
        assert_eq!(
            recall_transcript_status(Some("failed")),
            transcription_types::Status::Failed
        );
        assert_eq!(
            recall_transcript_status(None),
            transcription_types::Status::Queued
        );
        assert_eq!(
            recall_transcript_status(Some("unknown")),
            transcription_types::Status::Queued
        );
    }

    // ── coalesce_entries ──────────────────────────────────────────────────────

    #[test]
    fn coalesce_entries_empty_input_returns_empty() {
        assert!(coalesce_entries(vec![]).is_empty());
    }

    #[test]
    fn coalesce_entries_single_word_becomes_one_segment() {
        let entries = vec![word("Alice", "Hello", 0.0, 0.5)];
        let segments = coalesce_entries(entries);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "Hello");
        assert_eq!(segments[0].speaker, "Alice");
        assert_eq!(segments[0].start_ms, 0);
        assert_eq!(segments[0].end_ms, 500);
    }

    #[test]
    fn coalesce_entries_speaker_change_splits_into_two_segments() {
        let entries = vec![
            word("Alice", "Hello", 0.0, 0.5),
            word("Bob", "Goodbye", 1.0, 1.5),
        ];
        let segments = coalesce_entries(entries);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].speaker, "Alice");
        assert_eq!(segments[0].text, "Hello");
        assert_eq!(segments[1].speaker, "Bob");
        assert_eq!(segments[1].text, "Goodbye");
    }

    #[test]
    fn coalesce_entries_gap_over_threshold_splits_same_speaker() {
        // Alice speaks at 0-0.5 s, then again at 3.0 s — gap exceeds 1.5 s.
        let entries = vec![
            word("Alice", "First", 0.0, 0.5),
            word("Alice", "Second", 3.0, 3.5),
        ];
        let segments = coalesce_entries(entries);
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "First");
        assert_eq!(segments[1].text, "Second");
    }

    #[test]
    fn coalesce_entries_small_gap_keeps_same_speaker_together() {
        // Gap is 0.2 s — below the 1.5 s threshold, should merge into one segment.
        let entries = vec![
            word("Alice", "One", 0.0, 0.5),
            word("Alice", "Two", 0.7, 1.2),
        ];
        let segments = coalesce_entries(entries);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "One Two");
    }

    #[test]
    fn coalesce_entries_unknown_speaker_uses_fallback_label() {
        let entry = ParticipantEntry {
            participant: Participant {
                id: None,
                name: None,
                is_host: None,
                platform: None,
                email: None,
                extra_data: None,
            },
            language_code: None,
            words: vec![TranscriptWord {
                text: "Test".to_string(),
                start_timestamp: Timestamp {
                    absolute: None,
                    relative: Some(0.0),
                },
                end_timestamp: Timestamp {
                    absolute: None,
                    relative: Some(0.5),
                },
            }],
        };
        let segments = coalesce_entries(vec![entry]);
        assert_eq!(segments[0].speaker, "Unknown");
    }

    // ── HTTP — create_bot ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn create_bot_returns_ok_on_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/bot/")
            .with_status(201)
            .with_header("content-type", "application/json")
            .with_body(r#"{"id":"bot-test-123"}"#)
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider
            .create_bot("session-abc", "https://zoom.us/j/123", "Bot")
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, "bot-test-123");
    }

    #[tokio::test]
    async fn create_bot_returns_err_on_non_2xx() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/bot/")
            .with_status(422)
            .with_body("invalid meeting url")
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider.create_bot("session-abc", "bad-url", "Bot").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error_kind,
            DomainErrorKind::External(ExternalErrorKind::Other(_))
        ));
    }

    #[tokio::test]
    async fn create_bot_returns_err_on_invalid_json_response() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("POST", "/bot/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json")
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider
            .create_bot("session-abc", "https://zoom.us/j/123", "Bot")
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error_kind,
            DomainErrorKind::External(ExternalErrorKind::Other(_))
        ));
    }

    // ── HTTP — get_async_transcript ───────────────────────────────────────────

    #[tokio::test]
    async fn get_async_transcript_returns_ok_on_success() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/transcript/trans-123/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{"id":"trans-123","status":{"code":"complete"},"data":{"download_url":null}}"#,
            )
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider.get_async_transcript("trans-123").await;

        assert!(result.is_ok());
        let meta = result.unwrap();
        assert_eq!(meta.id, "trans-123");
        assert_eq!(meta.status_str(), Some("complete"));
    }

    #[tokio::test]
    async fn get_async_transcript_returns_err_on_non_2xx() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/transcript/missing/")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider.get_async_transcript("missing").await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_async_transcript_returns_err_on_invalid_json() {
        let mut server = mockito::Server::new_async().await;
        let _mock = server
            .mock("GET", "/transcript/bad-json/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body("not json at all")
            .create_async()
            .await;

        let provider = test_provider(&server.url());
        let result = provider.get_async_transcript("bad-json").await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err().error_kind,
            DomainErrorKind::External(ExternalErrorKind::Other(_))
        ));
    }
}
