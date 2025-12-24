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

// =============================================================================
// LeMUR API Types
// =============================================================================

/// Request for LeMUR custom task
#[derive(Debug, Serialize)]
pub struct LemurTaskRequest {
    /// Transcript IDs to analyze
    pub transcript_ids: Vec<String>,
    /// Custom prompt for the task
    pub prompt: String,
    /// Model to use (e.g., "anthropic/claude-sonnet-4-20250514")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_model: Option<String>,
    /// Maximum output size in tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_size: Option<i32>,
}

/// Response from LeMUR task
#[derive(Debug, Deserialize)]
pub struct LemurTaskResponse {
    /// Unique request ID
    pub request_id: String,
    /// The generated response text (may be JSON)
    pub response: String,
    /// Usage statistics (optional)
    #[serde(default)]
    pub usage: Option<LemurUsage>,
}

/// LeMUR usage statistics
#[derive(Debug, Deserialize, Default)]
pub struct LemurUsage {
    /// Input token count
    pub input_tokens: Option<i32>,
    /// Output token count
    pub output_tokens: Option<i32>,
}

/// Extracted action from LeMUR analysis
/// Actions have a single assignee responsible for completion
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtractedAction {
    /// Clear description of the action/task
    pub content: String,
    /// Original quote from the transcript
    pub source_text: String,
    /// Speaker label (e.g., "Speaker A", "Speaker B")
    pub stated_by_speaker: String,
    /// Who should complete: "coach" or "coachee"
    pub assigned_to_role: String,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
    /// Start time in milliseconds (optional)
    #[serde(default)]
    pub start_time_ms: Option<i64>,
    /// End time in milliseconds (optional)
    #[serde(default)]
    pub end_time_ms: Option<i64>,
}

/// Extracted agreement from LeMUR analysis
/// Agreements are mutual commitments with no single assignee
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExtractedAgreement {
    /// Description of what was agreed
    pub content: String,
    /// Original quote from the transcript
    pub source_text: String,
    /// Speaker who articulated the agreement
    pub stated_by_speaker: String,
    /// Confidence score (0.0 - 1.0)
    pub confidence: f64,
    /// Start time in milliseconds (optional)
    #[serde(default)]
    pub start_time_ms: Option<i64>,
    /// End time in milliseconds (optional)
    #[serde(default)]
    pub end_time_ms: Option<i64>,
}

/// Combined LeMUR extraction response
#[derive(Debug, Serialize, Deserialize)]
pub struct LemurExtractionResponse {
    /// Extracted actions with assignees
    pub actions: Vec<ExtractedAction>,
    /// Extracted mutual agreements
    pub agreements: Vec<ExtractedAgreement>,
}

/// Coaching-focused summary structure
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct CoachingSummary {
    /// Goals discussed during the session
    pub goals_discussed: Vec<String>,
    /// Progress made on previous goals/actions
    pub progress_made: Vec<String>,
    /// Challenges or obstacles identified
    pub challenges_identified: Vec<String>,
    /// Key insights or realizations
    pub key_insights: Vec<String>,
    /// Next steps and action items
    pub next_steps: Vec<String>,
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
            error!("AssemblyAI API: {}", error_text);
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
            error!("AssemblyAI API: {}", error_text);
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
            error!("AssemblyAI failed to delete transcript: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    // =========================================================================
    // LeMUR API Methods
    // =========================================================================

    /// Execute a custom LeMUR task with the given prompt
    pub async fn lemur_task(&self, request: LemurTaskRequest) -> Result<LemurTaskResponse, Error> {
        // LeMUR uses a different API path structure than the transcript API.
        // The base_url typically contains "/v2" for transcript endpoints, but
        // LeMUR endpoints use "/lemur/v3/..." without the "/v2" prefix.
        let lemur_base = self.base_url.trim_end_matches("/v2");
        let url = format!("{}/lemur/v3/generate/task", lemur_base);

        debug!(
            "Executing LeMUR task for {} transcript(s)",
            request.transcript_ids.len()
        );

        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to execute LeMUR task: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Network),
                }
            })?;

        if response.status().is_success() {
            let result: LemurTaskResponse = response.json().await.map_err(|e| {
                warn!("Failed to parse LeMUR response: {:?}", e);
                Error {
                    source: Some(Box::new(e)),
                    error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                        "Invalid response from LeMUR".to_string(),
                    )),
                }
            })?;
            debug!("LeMUR task completed: {}", result.request_id);
            Ok(result)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            error!("LeMUR API: {}", error_text);
            Err(Error {
                source: None,
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(error_text)),
            })
        }
    }

    /// Extract actions and agreements from a transcript using LeMUR
    ///
    /// Actions have a single assignee (coach or coachee) responsible for completion.
    /// Agreements are mutual commitments with no single assignee.
    pub async fn extract_actions_and_agreements(
        &self,
        transcript_id: &str,
        coach_name: &str,
        coachee_name: &str,
    ) -> Result<LemurExtractionResponse, Error> {
        let prompt = format!(
            r#"Analyze this coaching session transcript and extract ACTIONS and AGREEMENTS separately.

The coach is "{}" and the coachee is "{}".

## ACTIONS
Actions are tasks with a clear owner - one person is responsible for completing it.
- Has a concrete next step
- Someone commits to doing something specific
- Examples: "I'll send you that article", "Let me schedule a follow-up meeting"

## AGREEMENTS
Agreements are mutual commitments or shared understandings.
- Both parties agree to something together
- No single assignee - it's a shared commitment
- Examples: "We agreed to focus on leadership skills", "We'll meet bi-weekly going forward"

## Output Format
Return a JSON object with this exact structure:
{{
  "actions": [
    {{
      "content": "Clear description of the action",
      "source_text": "Exact quote from transcript",
      "stated_by_speaker": "Speaker A",
      "assigned_to_role": "coach",
      "confidence": 0.95,
      "start_time_ms": null,
      "end_time_ms": null
    }}
  ],
  "agreements": [
    {{
      "content": "Description of what was agreed",
      "source_text": "Exact quote from transcript",
      "stated_by_speaker": "Speaker B",
      "confidence": 0.90,
      "start_time_ms": null,
      "end_time_ms": null
    }}
  ]
}}

For assigned_to_role, use "coach" or "coachee" based on who should complete the action.
For stated_by_speaker, use the speaker label from the transcript (e.g., "Speaker A", "Speaker B").
Return ONLY valid JSON, no markdown or explanation."#,
            coach_name, coachee_name
        );

        let request = LemurTaskRequest {
            transcript_ids: vec![transcript_id.to_string()],
            prompt,
            final_model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            max_output_size: Some(4000),
        };

        let response = self.lemur_task(request).await?;

        // Parse the JSON response
        serde_json::from_str(&response.response).map_err(|e| {
            warn!(
                "Failed to parse LeMUR extraction response: {:?}, response: {}",
                e, response.response
            );
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                    "Invalid JSON from LeMUR extraction".to_string(),
                )),
            }
        })
    }

    /// Generate a coaching-focused summary using LeMUR
    pub async fn generate_coaching_summary(
        &self,
        transcript_id: &str,
        coach_name: &str,
        coachee_name: &str,
    ) -> Result<CoachingSummary, Error> {
        let prompt = format!(
            r#"Analyze this coaching session between "{}" (coach) and "{}" (coachee).

Create a structured summary with these sections:

1. **Goals Discussed**: What goals or objectives were talked about?
2. **Progress Made**: Any progress on previous goals or actions?
3. **Challenges Identified**: Obstacles, blockers, or difficulties mentioned?
4. **Key Insights**: Important realizations, aha moments, or learnings?
5. **Next Steps**: What happens next? Action items and plans?

Return a JSON object with exactly this structure:
{{
  "goals_discussed": ["Goal 1", "Goal 2"],
  "progress_made": ["Progress item 1"],
  "challenges_identified": ["Challenge 1"],
  "key_insights": ["Insight 1"],
  "next_steps": ["Next step 1", "Next step 2"]
}}

Guidelines:
- Keep each item to 1-2 concise sentences
- Use the coachee's perspective where appropriate
- Include 1-5 items per section (empty array if nothing applies)
- Return ONLY valid JSON, no markdown or explanation"#,
            coach_name, coachee_name
        );

        let request = LemurTaskRequest {
            transcript_ids: vec![transcript_id.to_string()],
            prompt,
            final_model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            max_output_size: Some(2000),
        };

        let response = self.lemur_task(request).await?;

        // Parse the JSON response
        serde_json::from_str(&response.response).map_err(|e| {
            warn!(
                "Failed to parse LeMUR summary response: {:?}, response: {}",
                e, response.response
            );
            Error {
                source: Some(Box::new(e)),
                error_kind: DomainErrorKind::External(ExternalErrorKind::Other(
                    "Invalid JSON from LeMUR summary".to_string(),
                )),
            }
        })
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
