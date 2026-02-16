//! AssemblyAI API client for transcription services.
//!
//! This module provides an HTTP client for interacting with the AssemblyAI API
//! to transcribe meeting recordings with speaker diarization and AI features.

use crate::error::{DomainErrorKind, Error, ExternalErrorKind, InternalErrorKind};
use async_trait::async_trait;
use log::*;
use meeting_ai::traits::{analysis as analysis_trait, transcription as transcription_trait};
use meeting_ai::types::{analysis, transcription};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

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
    /// Name of the person assigned (extracted from transcript, e.g., "Jim", "Sarah")
    #[serde(default)]
    pub assigned_to_name: Option<String>,
    /// Due date in ISO 8601 format (e.g., "2025-01-15") if mentioned in transcript
    #[serde(default)]
    pub due_by: Option<String>,
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
        // Get today's date for resolving relative dates in the transcript
        let now = chrono::Utc::now();
        let today = now.format("%Y-%m-%d").to_string();
        let day_of_week = now.format("%A").to_string(); // e.g., "Tuesday"

        // Pre-calculate common relative dates to help LLM accuracy
        let in_1_week = (now + chrono::Duration::days(7)).format("%Y-%m-%d");
        let in_2_weeks = (now + chrono::Duration::days(14)).format("%Y-%m-%d");
        let in_3_weeks = (now + chrono::Duration::days(21)).format("%Y-%m-%d");
        let in_4_weeks = (now + chrono::Duration::days(28)).format("%Y-%m-%d");
        let in_1_month = (now + chrono::Months::new(1)).format("%Y-%m-%d");
        let in_2_months = (now + chrono::Months::new(2)).format("%Y-%m-%d");
        let in_3_months = (now + chrono::Months::new(3)).format("%Y-%m-%d");
        let in_6_months = (now + chrono::Months::new(6)).format("%Y-%m-%d");
        let in_1_year = (now + chrono::Months::new(12)).format("%Y-%m-%d");

        let prompt = format!(
            r#"Analyze this coaching session transcript and extract ACTIONS and AGREEMENTS separately.

The coach is "{coach}" and the coachee is "{coachee}".
Today is {day_of_week}, {today}. Use these pre-calculated dates for relative deadlines:
- "in 1 week" / "in a week" = {in_1_week}
- "in 2 weeks" / "in two weeks" = {in_2_weeks}
- "in 3 weeks" / "in three weeks" = {in_3_weeks}
- "in 4 weeks" / "in four weeks" = {in_4_weeks}
- "in 1 month" / "in a month" = {in_1_month}
- "in 2 months" / "in two months" = {in_2_months}
- "in 3 months" / "in three months" = {in_3_months}
- "in 6 months" / "in six months" = {in_6_months}
- "in 1 year" / "in 12 months" = {in_1_year}
- "next Wednesday" = the upcoming Wednesday after today
- "by Friday" = the upcoming Friday

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

## EXTRACTING NAMES AND DUE DATES
For each action:
- If a specific person's name is mentioned as responsible, capture it in assigned_to_name (e.g., "Jim will...", "Sarah needs to...")
- If a due date or deadline is mentioned (e.g., "by Friday", "before January 15", "by next Wednesday"), extract it as due_by in ISO 8601 format (YYYY-MM-DD). Convert relative dates to absolute dates using today's date.

## Output Format
Return a JSON object with this exact structure:
{{
  "actions": [
    {{
      "content": "Clear description of the action",
      "source_text": "Exact quote from transcript",
      "stated_by_speaker": "Speaker A",
      "assigned_to_role": "coach",
      "assigned_to_name": "Jim",
      "due_by": "2025-01-15",
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
For assigned_to_name, use the exact name mentioned in the transcript if one was stated, otherwise null.
For due_by, use ISO 8601 date format (YYYY-MM-DD) if a deadline was mentioned, otherwise null.
For stated_by_speaker, use the speaker label from the transcript (e.g., "Speaker A", "Speaker B").
Return ONLY valid JSON, no markdown or explanation."#,
            coach = coach_name,
            coachee = coachee_name,
            day_of_week = day_of_week,
            today = today,
            in_1_week = in_1_week,
            in_2_weeks = in_2_weeks,
            in_3_weeks = in_3_weeks,
            in_4_weeks = in_4_weeks,
            in_1_month = in_1_month,
            in_2_months = in_2_months,
            in_3_months = in_3_months,
            in_6_months = in_6_months,
            in_1_year = in_1_year
        );

        let request = LemurTaskRequest {
            transcript_ids: vec![transcript_id.to_string()],
            prompt,
            final_model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            max_output_size: Some(4000),
        };

        let response = self.lemur_task(request).await?;

        // Strip markdown code blocks if present (LeMUR sometimes wraps JSON in ```json ... ```)
        let json_str = response.response.trim();
        let json_str = if json_str.starts_with("```") {
            // Find the end of the first line (after ```json or ```)
            let start = json_str.find('\n').map(|i| i + 1).unwrap_or(0);
            // Find the closing ```
            let end = json_str.rfind("```").unwrap_or(json_str.len());
            json_str[start..end].trim()
        } else {
            json_str
        };

        debug!("LeMUR extraction raw JSON: {}", json_str);

        // Parse the JSON response
        serde_json::from_str(json_str).map_err(|e| {
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
    ///
    /// The `speaker_mapping` parameter maps speaker labels (e.g., "A", "B") to names
    /// to help LeMUR understand who is speaking in the transcript.
    pub async fn generate_coaching_summary(
        &self,
        transcript_id: &str,
        coach_name: &str,
        coachee_name: &str,
        speaker_mapping: Option<&std::collections::HashMap<String, String>>,
    ) -> Result<CoachingSummary, Error> {
        // Build speaker context if mapping is provided
        let speaker_context = speaker_mapping
            .map(|mapping| {
                let speaker_info: Vec<String> = mapping
                    .iter()
                    .map(|(label, name)| format!("Speaker {} is {}", label, name))
                    .collect();
                if !speaker_info.is_empty() {
                    format!("\n\nSpeaker identification: {}", speaker_info.join(", "))
                } else {
                    String::new()
                }
            })
            .unwrap_or_default();

        let prompt = format!(
            r#"Analyze this coaching session between "{}" (coach) and "{}" (coachee).{}

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
            coach_name, coachee_name, speaker_context
        );

        let request = LemurTaskRequest {
            transcript_ids: vec![transcript_id.to_string()],
            prompt,
            final_model: Some("anthropic/claude-sonnet-4-20250514".to_string()),
            max_output_size: Some(2000),
        };

        let response = self.lemur_task(request).await?;

        // Strip markdown code blocks if present (LeMUR sometimes wraps JSON in ```json ... ```)
        let json_str = response.response.trim();
        let json_str = if json_str.starts_with("```") {
            let start = json_str.find('\n').map(|i| i + 1).unwrap_or(0);
            let end = json_str.rfind("```").unwrap_or(json_str.len());
            json_str[start..end].trim()
        } else {
            json_str
        };

        // Parse the JSON response
        serde_json::from_str(json_str).map_err(|e| {
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

// Implement the meeting-ai transcription::Provider trait
#[async_trait]
impl transcription_trait::Provider for AssemblyAiClient {
    async fn create_transcription(
        &self,
        config: transcription::Config,
    ) -> std::result::Result<transcription::Transcription, meeting_ai::Error> {
        let request = CreateTranscriptRequest {
            audio_url: config.media_url,
            webhook_url: config.webhook_url.clone(),
            webhook_auth_header_name: config
                .webhook_url
                .as_ref()
                .map(|_| "X-Webhook-Secret".to_string()),
            webhook_auth_header_value: None,
            speaker_labels: config.enable_speaker_labels,
            sentiment_analysis: config.enable_sentiment_analysis,
            auto_chapters: config.enable_auto_chapters,
            entity_detection: config.enable_entity_detection,
        };

        let response = self
            .create_transcript(request)
            .await
            .map_err(|e| meeting_ai::Error::Provider(e.to_string()))?;

        Ok(map_transcript_response(response))
    }

    async fn get_transcription(
        &self,
        transcription_id: &str,
    ) -> std::result::Result<transcription::Transcription, meeting_ai::Error> {
        let response = self
            .get_transcript(transcription_id)
            .await
            .map_err(|e| meeting_ai::Error::Provider(e.to_string()))?;

        Ok(map_transcript_response(response))
    }

    async fn delete_transcription(
        &self,
        transcription_id: &str,
    ) -> std::result::Result<(), meeting_ai::Error> {
        self.delete_transcript(transcription_id)
            .await
            .map_err(|e| meeting_ai::Error::Provider(e.to_string()))
    }

    fn provider_id(&self) -> &str {
        "assemblyai"
    }

    async fn verify_credentials(&self) -> std::result::Result<bool, meeting_ai::Error> {
        self.verify_api_key()
            .await
            .map_err(|e| meeting_ai::Error::Authentication(e.to_string()))
    }
}

// Implement the meeting-ai analysis::Provider trait
#[async_trait]
impl analysis_trait::Provider for AssemblyAiClient {
    async fn analyze(
        &self,
        config: analysis::Config,
    ) -> std::result::Result<analysis::Result, meeting_ai::Error> {
        // For now, we'll use a simplified implementation
        // In production, you'd want to use LeMUR or a more sophisticated extraction
        let _transcript_id = &config.transcript_id;

        // Create a basic result
        let result = analysis::Result {
            request_id: Uuid::new_v4().to_string(),
            resources: HashMap::new(),
            summary: if config.generate_summary {
                Some(analysis::Summary {
                    overview: "Meeting summary placeholder".to_string(),
                    key_points: vec![],
                    goals: vec![],
                    challenges: vec![],
                    insights: vec![],
                    next_steps: vec![],
                    topics: vec![],
                })
            } else {
                None
            },
            token_usage: None,
        };

        Ok(result)
    }

    async fn custom_task(
        &self,
        transcript_id: &str,
        prompt: &str,
    ) -> std::result::Result<String, meeting_ai::Error> {
        let request = LemurTaskRequest {
            transcript_ids: vec![transcript_id.to_string()],
            prompt: prompt.to_string(),
            final_model: Some("default".to_string()),
            max_output_size: None,
        };

        let response = self
            .lemur_task(request)
            .await
            .map_err(|e| meeting_ai::Error::Provider(e.to_string()))?;

        Ok(response.response)
    }

    fn provider_id(&self) -> &str {
        "assemblyai"
    }

    async fn verify_credentials(&self) -> std::result::Result<bool, meeting_ai::Error> {
        self.verify_api_key()
            .await
            .map_err(|e| meeting_ai::Error::Authentication(e.to_string()))
    }
}

/// Map AssemblyAI transcript response to meeting-ai transcription
fn map_transcript_response(response: TranscriptResponse) -> transcription::Transcription {
    transcription::Transcription {
        id: response.id,
        status: map_transcript_status(response.status),
        text: response.text,
        words: response
            .words
            .unwrap_or_default()
            .into_iter()
            .map(map_word)
            .collect(),
        segments: response
            .utterances
            .unwrap_or_default()
            .into_iter()
            .map(map_segment)
            .collect(),
        chapters: response
            .chapters
            .unwrap_or_default()
            .into_iter()
            .map(map_chapter)
            .collect(),
        sentiment_analysis: response
            .sentiment_analysis_results
            .unwrap_or_default()
            .into_iter()
            .map(map_sentiment)
            .collect(),
        confidence: response.confidence,
        duration_seconds: response.audio_duration,
        language_code: None,
        speaker_count: None,
        error_message: response.error,
    }
}

/// Map AssemblyAI status to meeting-ai status
fn map_transcript_status(status: TranscriptStatus) -> transcription::Status {
    match status {
        TranscriptStatus::Queued => transcription::Status::Queued,
        TranscriptStatus::Processing => transcription::Status::Processing,
        TranscriptStatus::Completed => transcription::Status::Completed,
        TranscriptStatus::Error => transcription::Status::Failed,
    }
}

/// Map AssemblyAI word to meeting-ai word
fn map_word(word: Word) -> transcription::Word {
    transcription::Word {
        text: word.text,
        start_ms: word.start,
        end_ms: word.end,
        confidence: word.confidence,
        speaker: word.speaker,
    }
}

/// Map AssemblyAI utterance to meeting-ai segment
fn map_segment(utterance: Utterance) -> transcription::Segment {
    transcription::Segment {
        text: utterance.text,
        speaker: utterance.speaker,
        start_ms: utterance.start,
        end_ms: utterance.end,
        confidence: utterance.confidence,
        words: utterance
            .words
            .unwrap_or_default()
            .into_iter()
            .map(map_word)
            .collect(),
    }
}

/// Map AssemblyAI chapter to meeting-ai chapter
fn map_chapter(chapter: Chapter) -> transcription::Chapter {
    transcription::Chapter {
        title: chapter.headline,
        summary: chapter.summary,
        gist: chapter.gist,
        start_ms: chapter.start,
        end_ms: chapter.end,
    }
}

/// Map AssemblyAI sentiment result to meeting-ai sentiment analysis
fn map_sentiment(result: SentimentResult) -> transcription::SentimentAnalysis {
    transcription::SentimentAnalysis {
        text: result.text,
        sentiment: map_sentiment_value(result.sentiment),
        confidence: result.confidence,
        start_ms: result.start,
        end_ms: result.end,
        speaker: result.speaker,
    }
}

/// Map AssemblyAI sentiment to meeting-ai sentiment
fn map_sentiment_value(sentiment: Sentiment) -> transcription::Sentiment {
    match sentiment {
        Sentiment::Positive => transcription::Sentiment::Positive,
        Sentiment::Neutral => transcription::Sentiment::Neutral,
        Sentiment::Negative => transcription::Sentiment::Negative,
    }
}
