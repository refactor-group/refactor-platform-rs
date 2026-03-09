# Meeting AI Abstraction Layer Design

**Status:** Design Phase
**Date:** 2026-01-31 (Updated)
**Author:** Caleb Bourg & Claude
**Approach:** Hybrid (Standalone crate + Domain implementations)
**Dependencies:** `meeting-auth`, `meeting-manager` (see `meeting-and-auth-abstraction-layers.md`)

## Executive Summary

This document proposes a comprehensive abstraction layer for meeting AI workflows that decouples business logic from specific third-party AI providers (Recall.ai, AssemblyAI, etc.). The design focuses on **recording bot deployment, transcription, and AI analysis** workflows without being locked into specific vendors.

**Important:** This plan depends on and integrates with the `meeting-auth` and `meeting-manager` crates defined in `meeting-and-auth-abstraction-layers.md`. OAuth authentication, meeting creation, HTTP client configuration, and webhook signature validation are handled by those crates and are **NOT** duplicated here.

## Current State Analysis

### Existing Workflow

The current implementation in `refactor-platform-rs` follows this flow:

1. **OAuth Authentication** (Google OAuth) → Handled by `meeting-auth` crate *(see other plan)*
2. **Meeting Creation** (Google Meet API) → Handled by `meeting-manager` crate *(see other plan)*
3. **Recording** (Recall.ai) → Deploy bot to join and record meeting *(this plan)*
4. **Transcription** (AssemblyAI) → Transcribe recording with speaker diarization *(this plan)*
5. **AI Analysis** (AssemblyAI LeMUR) → Extract actions, agreements, generate summaries *(this plan)*
6. **Webhook Processing** → Handle async status updates from AI providers *(this plan)*

### Key Files for AI Workflows

**This plan covers:**
- `domain/src/gateway/recall_ai.rs` - Recall.ai bot management
- `domain/src/gateway/assembly_ai.rs` - Transcription and LeMUR analysis
- `web/src/controller/meeting_recording_controller.rs` - Recording endpoints
- `web/src/controller/transcription_controller.rs` - Transcription endpoints
- `web/src/controller/webhook_controller.rs` - Webhook handlers (AI provider events)

**Handled by meeting-auth + meeting-manager:**
- `domain/src/gateway/google_oauth.rs` - OAuth and Meet API *(covered in other plan)*
- OAuth token management, refresh, and storage
- Meeting space creation APIs

### Problems with Current Design

1. **Tight coupling** - Direct dependencies on specific providers
2. **No provider flexibility** - Can't swap Recall.ai for alternatives (Skribby, Meeting BaaS, etc.)
3. **Testing challenges** - Hard to mock providers for tests
4. **Code duplication** - Similar patterns repeated for each provider
5. **No standardization** - Each provider has different error handling, types, etc.

## Industry Research Findings

### Alternative Providers

**Meeting Bots:**
- Recall.ai ($1,000/month + $1/hour)
- Skribby ($0.35/hour, no platform fee)
- Meeting BaaS ($0.69/hour)
- Nylas Notetaker ($0.70/hour)
- Attendee (open source)

**Transcription:**
- AssemblyAI (current)
- Deepgram
- AWS Transcribe
- Azure Speech Services
- OpenAI Whisper (self-hosted)

**AI Analysis:**
- AssemblyAI LeMUR (current)
- OpenAI GPT-4
- Anthropic Claude
- Custom fine-tuned models

### Best Practices (2025)

1. **Hybrid Architecture** - Combine ASR with LLM-based semantic understanding
2. **Real-time + Batch** - Support both live transcription and post-meeting processing
3. **Event-Driven** - Use webhooks for async updates
4. **Context-Aware** - Build organizational knowledge graphs for better AI understanding
5. **Multi-modal** - Include visual content from screen shares
6. **Governance-First** - Enterprise-grade data protection and compliance

## Proposed Architecture

### Hybrid Approach

**Standalone Crate** (`meeting-ai`)
- Core trait definitions
- Common types and errors
- Provider-agnostic interfaces
- Published to crates.io for reusability

**Domain Layer** (`domain/src/gateway/`)
- Concrete implementations (Recall.ai, AssemblyAI, etc.)
- Business logic specific to refactor-platform
- Integration with existing entity models

### Design Principles

1. **Provider Agnostic** - Traits hide implementation details
2. **Async-First** - All operations are async (tokio/async-std compatible)
3. **Type-Safe** - Rich enum types prevent invalid states
4. **Error Handling** - Unified error types with provider-specific context
5. **Event-Driven** - Webhook system for async state changes
6. **Testable** - Easy to mock providers for unit tests
7. **Extensible** - New providers implement existing traits

## Core Abstractions

### Trait Hierarchy

**This crate provides:**

```
RecordingBotProvider          (Meeting bots - record meetings)
    ├── RecallAiProvider
    ├── SkribbyProvider
    └── MeetingBaasProvider

TranscriptionProvider         (Speech-to-text)
    ├── AssemblyAiProvider
    ├── DeepgramProvider
    └── WhisperProvider

AnalysisProvider           (Action extraction, summaries)
    ├── LemurProvider
    ├── OpenAiProvider
    └── ClaudeProvider

WebhookHandler               (Event processing)
    └── Custom implementations per app
```

**Provided by meeting-auth + meeting-manager crates:**

```
OAuthProvider                (OAuth 2.0 authentication)
    ├── GoogleOAuthProvider
    ├── ZoomOAuthProvider
    └── MicrosoftOAuthProvider

MeetingClient               (Meeting space creation)
    ├── GoogleMeetClient
    ├── ZoomMeetingClient
    └── TeamsMeetingClient

ProviderAuth                (API key authentication)
    ├── ApiKeyAuth
    └── BearerTokenAuth

WebhookValidator            (Webhook signature validation)
    └── HmacWebhookValidator
```

## Complete Trait Definitions

> **Scope Note:** This section defines traits for AI-specific operations (recording bots, transcription, AI analysis). OAuth authentication, meeting creation, HTTP client building, and webhook signature validation are handled by `meeting-auth` and `meeting-manager` crates (see `meeting-and-auth-abstraction-layers.md`).

### 1. Core Types

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Universal error type that abstracts provider-specific errors into common variants.
/// This unified error type eliminates the need for controller-level error mapping
/// and provides consistent error handling across all meeting AI providers.
/// All provider implementations should map their native errors to these variants,
/// preserving context while maintaining a provider-agnostic interface.
#[derive(Debug, Error)]
pub enum Error {
    /// OAuth or API key authentication failures. Indicates credentials are invalid,
    /// expired, or lack necessary permissions. Clients should prompt for re-authentication.
    #[error("Authentication failed: {0}")]
    Authentication(String),

    /// Network connectivity issues, DNS failures, or connection timeouts.
    /// These errors are typically transient and may benefit from retry logic.
    #[error("Network error: {0}")]
    Network(String),

    /// Invalid parameters, missing required fields, or malformed configuration.
    /// These errors indicate a programming error and should be fixed at development time.
    #[error("Invalid configuration: {0}")]
    Configuration(String),

    /// Provider-specific business logic errors (e.g., meeting not found, bot rejected).
    /// These are provider-level failures that may require user intervention or workflow changes.
    #[error("Provider error: {0}")]
    Provider(String),

    /// Operation exceeded the configured or provider-enforced timeout period.
    /// Consider increasing timeout limits or breaking operations into smaller chunks.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Requested resource (bot, transcription, meeting) does not exist.
    /// Verify IDs are correct and the resource hasn't been deleted.
    #[error("Not found: {0}")]
    NotFound(String),

    /// Provider rate limit exceeded. Clients must wait before retrying.
    /// Respect the retry_after_seconds to avoid further rate limiting or API suspension.
    #[error("Rate limited: retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    /// Failed to serialize data to JSON. Indicates type incompatibility or invalid data.
    /// Usually occurs when adding custom resources to AnalysisResult.
    #[error("Serialization error: {0}")]
    Serialization(String),

    /// Failed to deserialize JSON data to expected type. Indicates type mismatch.
    /// Usually occurs when extracting resources with get_resources::<T>().
    #[error("Deserialization error: {0}")]
    Deserialization(String),

    /// Catch-all for errors that don't fit other categories.
    /// Used for unexpected errors or provider-specific edge cases.
    #[error("Other error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}
```

### 2. Recording Bot Provider

> **Note:** OAuth and meeting creation are handled by the `meeting-auth` and `meeting-manager` crates (see `meeting-and-auth-abstraction-layers.md`). This crate focuses on AI-specific operations: bot deployment, transcription, and analysis.

```rust
/// Lifecycle status of a recording bot joining and recording a meeting.
/// Bots transition through states from Pending → Joining → InMeeting → Recording → Completed.
/// Failed status may occur at any point due to auth issues, meeting not found, or bot rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BotStatus {
    Pending,
    Joining,
    WaitingRoom,
    InMeeting,
    Recording,
    Processing,
    Completed,
    Failed,
}

/// Media artifacts produced by a recording bot after meeting ends.
/// URLs typically expire after 24-48 hours, so download and persist files
/// or trigger transcription immediately. All URLs are pre-signed for direct download.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingArtifacts {
    pub video_url: Option<String>,
    pub audio_url: Option<String>,
    pub duration_seconds: Option<i32>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub file_size_bytes: Option<u64>,
    pub metadata: HashMap<String, String>,
}

/// Complete information about a recording bot's state and outputs.
/// Monitor status field and artifacts become available when status reaches Completed.
/// Check error_message when status is Failed to diagnose issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotInfo {
    pub id: String,
    pub meeting_url: String,
    pub status: BotStatus,
    pub artifacts: Option<RecordingArtifacts>,
    pub error_message: Option<String>,
    pub status_history: Vec<BotStatusChange>,
}

/// Historical record of bot status transitions.
/// Useful for debugging, analytics, and understanding bot lifecycle.
/// Providers send these via webhooks or return in get_bot_status calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotStatusChange {
    pub status: BotStatus,
    pub timestamp: DateTime<Utc>,
    pub message: Option<String>,
}

/// Configuration for deploying a recording bot to a meeting.
/// Provider-specific options (e.g., video quality, streaming endpoints) go in provider_options.
/// Set webhook_url to receive async status updates; without it, you must poll get_bot_status.
#[derive(Debug, Clone)]
pub struct BotConfig {
    pub meeting_url: String,
    pub bot_name: String,
    pub webhook_url: Option<String>,
    pub record_video: bool,
    pub record_audio: bool,
    pub enable_realtime_transcription: bool,
    pub provider_options: HashMap<String, String>,
}

/// Optional filters for listing bots when querying bot history.
/// Useful for debugging, showing user's bot history, or finding active bots.
/// Unset fields are not applied as filters (returns all matches).
#[derive(Debug, Clone, Default)]
pub struct BotFilters {
    pub status: Option<BotStatus>,
    pub meeting_url: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
}

/// Abstraction for meeting bot services that join meetings to record.
/// Implementations deploy virtual participants to meetings, record video/audio,
/// and return media artifacts. Supports providers like Recall.ai, Skribby, Meeting BaaS.
/// This trait enables cost optimization by swapping providers without code changes.
#[async_trait]
pub trait RecordingBotProvider: Send + Sync {
    /// Deploy a bot to join and record a meeting.
    /// Bot immediately begins joining process; track progress via webhooks or polling.
    /// Returns BotInfo with id for subsequent status checks and bot control.
    async fn create_bot(&self, config: BotConfig) -> Result<BotInfo, Error>;

    /// Retrieve current status and available artifacts for a bot.
    /// Poll this endpoint if webhook_url was not configured during creation.
    /// Artifacts populate when status reaches Completed or Processing.
    async fn get_bot_status(&self, bot_id: &str) -> Result<BotInfo, Error>;

    /// Immediately remove bot from meeting and stop recording.
    /// Use when user manually ends recording early or cancels session.
    /// Partial recordings may still be available depending on provider.
    async fn stop_bot(&self, bot_id: &str) -> Result<(), Error>;

    /// Query all bots with optional filters (status, meeting URL, date range).
    /// Useful for admin dashboards, debugging, or finding bots by meeting.
    /// Large result sets may require pagination (implement in provider_options).
    async fn list_bots(&self, filters: Option<BotFilters>) -> Result<Vec<BotInfo>, Error>;

    /// Return unique identifier for this provider (e.g., "recall_ai", "skribby").
    /// Used for logging, cost tracking, and selecting providers at runtime.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    /// Call during user onboarding or settings updates to provide immediate feedback.
    /// Returns false if credentials are invalid, expired, or lack permissions.
    async fn verify_credentials(&self) -> Result<bool, Error>;
}
```

### 4. Transcription Provider

```rust
/// Processing status of a speech-to-text transcription job.
/// Jobs typically progress Queued → Processing → Completed within minutes.
/// Poll or use webhooks to monitor progress; avoid tight loops that waste API quota.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

/// Individual word with precise timing and speaker attribution.
/// Enables word-level highlighting, search, and navigation in transcript UIs.
/// Confidence scores help identify low-quality audio segments that may need review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub speaker: Option<String>,
}

/// Continuous speech segment (utterance) from a single speaker.
/// Represents natural speaking turns in conversation with speaker diarization.
/// Use segments for speaker attribution, conversation flow analysis, and UI display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub speaker: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub words: Vec<TranscriptWord>,
}

/// Auto-detected topical chapter with AI-generated summary.
/// Providers use NLP to identify topic changes and create logical sections.
/// Useful for long meetings to help users navigate to relevant discussions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChapter {
    pub title: String,
    pub summary: String,
    pub gist: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Emotional tone classification (positive, negative, neutral).
/// Use for conversation quality analysis, coaching feedback, or conflict detection.
/// Confidence below 0.7 suggests ambiguous emotional tone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
}

/// Sentiment analysis for a segment of the transcript.
/// Links emotional tone to specific text, speaker, and timestamp for contextual analysis.
/// Aggregate sentiment scores provide meeting mood indicators and communication insights.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentAnalysis {
    pub text: String,
    pub sentiment: Sentiment,
    pub confidence: f64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
}

/// Complete transcription with speech-to-text results and optional enhancements.
/// Fields populate based on enabled features in TranscriptionConfig.
/// Poll get_transcription until status is Completed or Failed before accessing results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcription {
    pub id: String,
    pub status: TranscriptionStatus,
    pub text: Option<String>,
    pub words: Vec<TranscriptWord>,
    pub segments: Vec<TranscriptSegment>,
    pub chapters: Vec<TranscriptChapter>,
    pub sentiment_analysis: Vec<SentimentAnalysis>,
    pub confidence: Option<f64>,
    pub duration_seconds: Option<i64>,
    pub language_code: Option<String>,
    pub speaker_count: Option<u32>,
    pub error_message: Option<String>,
}

/// Configuration for creating a transcription job.
/// Enable optional features (speaker labels, sentiment, chapters) via flags.
/// Set webhook_url to receive completion notification; otherwise poll get_transcription.
/// Provider_options allow vendor-specific tuning (e.g., custom vocabulary, punctuation).
#[derive(Debug, Clone)]
pub struct TranscriptionConfig {
    pub media_url: String,
    pub webhook_url: Option<String>,
    pub enable_speaker_labels: bool,
    pub enable_sentiment_analysis: bool,
    pub enable_auto_chapters: bool,
    pub enable_entity_detection: bool,
    pub language_code: Option<String>,
    pub provider_options: HashMap<String, String>,
}

/// Abstraction for speech-to-text transcription services.
/// Implementations convert audio/video to text with speaker diarization, timing,
/// and optional enhancements (sentiment, chapters). Supports AssemblyAI, Deepgram, Whisper.
/// This trait enables provider swapping for cost optimization and feature comparison.
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Start async transcription job for audio/video at media_url.
    /// Returns immediately with job ID; results available via get_transcription when complete.
    /// Media must be publicly accessible or use pre-signed URL with sufficient expiry.
    async fn create_transcription(&self, config: TranscriptionConfig) -> Result<Transcription, Error>;

    /// Retrieve transcription status and results by ID.
    /// Poll until status is Completed or Failed. Rate limit polling to avoid quota waste.
    /// All fields (words, segments, etc.) populate only when status is Completed.
    async fn get_transcription(&self, transcription_id: &str) -> Result<Transcription, Error>;

    /// Permanently delete transcription and associated data from provider storage.
    /// Use for GDPR compliance, data retention policies, or cleaning up test data.
    /// Some providers auto-delete after retention period (e.g., 30 days).
    async fn delete_transcription(&self, transcription_id: &str) -> Result<(), Error>;

    /// Return unique identifier for this provider (e.g., "assemblyai", "deepgram").
    /// Used for cost tracking, feature-specific logic, and provider selection.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    /// Call during user onboarding or settings updates for immediate validation feedback.
    /// Returns false if credentials invalid, expired, or lack transcription permissions.
    async fn verify_credentials(&self) -> Result<bool, Error>;
}
```

### 5. Resource Extraction System

The crate provides a flexible trait-based system for extracting application-specific resources
from meeting transcripts. Applications define their own resource types completely based on their
domain needs by implementing the `ExtractedResource` trait. The crate makes no assumptions about
what types of resources exist - enabling use cases from coaching sessions to medical consultations
to sales calls to project meetings.

```rust
/// Trait that all extractable resources must implement.
/// Provides common metadata interface while allowing completely custom fields and behavior.
/// Applications define domain-specific types (e.g., SeaORM models, DTOs, plain structs).
/// Examples: coaching actions, sales leads, medical diagnoses, project tasks, etc.
/// Type must be Clone, Send, Sync for async processing and Serialize/Deserialize for API responses.
pub trait ExtractedResource: Debug + Clone + Send + Sync + Serialize + DeserializeOwned {
    /// Unique identifier for the resource type defined by the application.
    /// Examples: "coaching_action", "sales_lead", "diagnosis", "task", "risk", "decision"
    /// Used for routing, storage, API responses, and distinguishing resource types.
    /// Must be consistent across all instances of this type.
    fn resource_type(&self) -> &'static str;

    /// Primary text content extracted from the transcript.
    /// Required for all resources; the core information being captured.
    /// This is the main human-readable description of what was extracted.
    fn content(&self) -> &str;

    /// AI confidence score (0.0-1.0) indicating extraction quality.
    /// Use to filter low-confidence extractions or flag items for human review.
    /// Scores below 0.7 typically indicate ambiguous phrasing requiring validation.
    fn confidence(&self) -> f64;

    /// Timestamp in milliseconds from meeting start where this was mentioned.
    /// None if resource spans multiple time ranges or timestamp unavailable.
    /// Enables linking resources back to specific moments in recording/transcript.
    fn timestamp_ms(&self) -> Option<i64>;
}

/// Key topic identified across the conversation with all mentions.
/// Relevance_score ranks topics by prominence; use for navigation and insights.
/// Mentions provide timestamps for jumping to topic discussions in recording.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTopic {
    pub name: String,
    pub relevance_score: f64,
    pub mentions: Vec<TopicMention>,
}

/// Single mention of a topic with context and timestamp.
/// Enables users to navigate to specific parts of the conversation.
/// Text provides snippet of surrounding discussion for context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicMention {
    pub text: String,
    pub timestamp_ms: i64,
}

/// Structured summary of meeting content generated by LLM.
/// Organizes conversation into logical categories for quick review and sharing.
/// Quality depends on transcript accuracy and prompt engineering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub overview: String,
    pub key_points: Vec<String>,
    pub goals: Vec<String>,
    pub challenges: Vec<String>,
    pub insights: Vec<String>,
    pub next_steps: Vec<String>,
    pub topics: Vec<ExtractedTopic>,
}

/// Complete result from AI analysis with flexible resource types.
/// Stores extracted resources grouped by type in a type-erased format for maximum flexibility.
/// Applications can extract any number and types of resources based on domain needs.
/// Use get_resources() for type-safe deserialization of specific resource types.
/// Token_usage helps track LLM costs and optimize prompts for efficiency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub request_id: String,
    /// Extracted resources grouped by resource_type (e.g., "coaching_action", "sales_lead").
    /// Stored as JSON values to support any application-defined resource types.
    /// Use get_resources::<T>() to deserialize resources of a specific type.
    pub resources: HashMap<String, Vec<serde_json::Value>>,
    pub summary: Option<MeetingSummary>,
    pub token_usage: Option<TokenUsage>,
}

impl AnalysisResult {
    /// Type-safe extraction of resources by type.
    /// Deserializes all resources matching the specified type T.
    /// Returns empty Vec if no resources of that type exist.
    /// Returns error if deserialization fails (indicates type mismatch).
    ///
    /// # Example
    /// ```
    /// let actions: Vec<CoachingAction> = result.get_resources()?;
    /// let leads: Vec<SalesLead> = result.get_resources()?;
    /// ```
    pub fn get_resources<T: ExtractedResource>(&self) -> Result<Vec<T>, Error> {
        let type_key = std::any::type_name::<T>()
            .split("::")
            .last()
            .unwrap_or("");

        self.resources
            .get(type_key)
            .map(|values| {
                values.iter()
                    .map(|v| serde_json::from_value(v.clone())
                        .map_err(|e| Error::Deserialization(e.to_string())))
                    .collect()
            })
            .unwrap_or_else(|| Ok(vec![]))
    }

    /// Add resources of a specific type to the result.
    /// Used by provider implementations to populate the result with extracted resources.
    pub fn add_resources<T: ExtractedResource>(&mut self, resources: Vec<T>) -> Result<(), Error> {
        if resources.is_empty() {
            return Ok(());
        }

        let type_key = resources[0].resource_type().to_string();
        let values: Result<Vec<serde_json::Value>, _> = resources.iter()
            .map(|r| serde_json::to_value(r)
                .map_err(|e| Error::Serialization(e.to_string())))
            .collect();

        self.resources.insert(type_key, values?);
        Ok(())
    }
}

/// LLM token consumption metrics for cost tracking and optimization.
/// Input_tokens = transcript + prompt; output_tokens = generated analysis.
/// Monitor these to optimize prompt length and detect cost anomalies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Configuration for LLM-powered transcript analysis.
/// Completely domain-agnostic - applications define extraction requirements via prompts.
/// Provide participant context to improve speaker attribution and name resolution.
/// Resource_types specifies what to extract (e.g., ["coaching_action", "sales_lead"]).
/// Custom_prompt provides detailed extraction instructions for the LLM.
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    pub transcript_id: String,
    pub participants: Vec<Participant>,
    /// Types of resources to extract (e.g., "coaching_action", "sales_lead", "diagnosis").
    /// Provider uses these to structure extraction prompts and organize results.
    pub resource_types: Vec<String>,
    pub generate_summary: bool,
    /// Custom prompt with domain-specific extraction instructions.
    /// Should describe what each resource type means and what fields to extract.
    /// Example: "Extract coaching_action items with assignee, due_date, and priority fields."
    pub extraction_prompt: String,
    pub model: Option<String>,
    pub provider_options: HashMap<String, String>,
}

/// Meeting participant with role and speaker label mapping.
/// Speaker_label links to transcription output (e.g., "Speaker A") for name resolution.
/// Role provides context to LLM for better resource attribution and entity resolution.
/// Examples: "coach"/"coachee", "doctor"/"patient", "sales_rep"/"prospect", "manager"/"employee"
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub name: String,
    pub role: Option<String>,
    pub speaker_label: Option<String>,
}

/// Abstraction for LLM-powered meeting transcript analysis.
/// Implementations use large language models to extract domain-specific resources
/// from transcripts. Supports AssemblyAI LeMUR, OpenAI GPT-4, Anthropic Claude.
/// This trait enables model comparison, cost optimization, and provider switching.
/// Completely domain-agnostic - applications define what to extract via AnalysisConfig.
#[async_trait]
pub trait AnalysisProvider: Send + Sync {
    /// Analyze transcript and extract structured resources based on config.
    /// Processing typically takes 10-60 seconds depending on transcript length and model.
    /// Returns domain-specific resources defined by application's extraction_prompt.
    /// Resources organized by type in AnalysisResult for type-safe deserialization.
    async fn analyze(&self, config: AnalysisConfig) -> Result<AnalysisResult, Error>;

    /// Run custom LLM prompt against transcript for domain-specific analysis.
    /// Use for specialized extractions not covered by standard analyze() method.
    /// Returns raw LLM response; parse result according to your prompt instructions.
    async fn custom_task(&self, transcript_id: &str, prompt: &str) -> Result<String, Error>;

    /// Return unique identifier for this provider (e.g., "lemur", "openai", "claude").
    /// Used for cost tracking, model-specific logic, and provider selection.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    /// Call during user onboarding or settings updates for immediate validation.
    /// Returns false if credentials invalid, expired, or lack analysis permissions.
    async fn verify_credentials(&self) -> Result<bool, Error>;
}
```

#### Application Integration Example

Complete example showing how applications define and extract custom resources:

```rust
// Application defines its own domain-specific resource types
use meeting_ai::{ExtractedResource, AnalysisProvider, AnalysisResult, AnalysisConfig};
use entity::actions; // SeaORM model

/// Application-specific action resource for coaching sessions.
/// Implements ExtractedResource to work with meeting-ai framework.
/// Can be directly converted to SeaORM model for database persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachingAction {
    pub id: Option<Id>,
    pub coaching_session_id: Id,
    pub user_id: Id,
    pub body: String,
    pub due_by: Option<DateTimeWithTimeZone>,
    // Fields from AI extraction
    pub confidence: f64,
    pub timestamp_ms: Option<i64>,
    pub source_text: String,
}

impl ExtractedResource for CoachingAction {
    fn resource_type(&self) -> &'static str { "coaching_action" }
    fn content(&self) -> &str { &self.body }
    fn confidence(&self) -> f64 { self.confidence }
    fn timestamp_ms(&self) -> Option<i64> { self.timestamp_ms }
}

/// Application's AI provider implementation.
/// Maps between provider's API and application's domain types.
pub struct SkribbyProvider {
    client: SkribbyClient,
}

#[async_trait]
impl AnalysisProvider for SkribbyProvider {
    async fn analyze(&self, config: AnalysisConfig) -> Result<AnalysisResult, Error> {
        // Call provider API with extraction prompt
        let response = self.client.analyze_transcript(
            &config.transcript_id,
            &config.extraction_prompt,
            &config.participants,
        ).await?;

        // Create result and populate with application-specific types
        let mut result = AnalysisResult {
            request_id: response.request_id,
            resources: HashMap::new(),
            summary: response.summary,
            token_usage: response.token_usage,
        };

        // Map provider response to application's CoachingAction type
        let actions: Vec<CoachingAction> = response.extracted_items
            .into_iter()
            .filter(|item| item.item_type == "coaching_action")
            .map(|item| CoachingAction {
                id: None,
                coaching_session_id: config.coaching_session_id,
                user_id: config.user_id,
                body: item.content,
                due_by: item.due_date.map(parse_datetime),
                confidence: item.confidence,
                timestamp_ms: item.timestamp_ms,
                source_text: item.source_text,
            })
            .collect();

        result.add_resources(actions)?;
        Ok(result)
    }

    fn provider_id(&self) -> &str { "skribby" }

    async fn verify_credentials(&self) -> Result<bool, Error> {
        self.client.verify_api_key().await
    }

    async fn custom_task(&self, transcript_id: &str, prompt: &str) -> Result<String, Error> {
        self.client.custom_query(transcript_id, prompt).await
    }
}

// Usage in controller
pub async fn analyze_coaching_session(
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    let provider = app_state.get_analysis_provider(user.id).await?;

    let config = AnalysisConfig {
        transcript_id: session.transcript_id.clone(),
        participants: vec![
            Participant {
                name: coach.full_name.clone(),
                role: Some("coach".to_string()),
                speaker_label: Some("Speaker A".to_string()),
            },
            Participant {
                name: coachee.full_name.clone(),
                role: Some("coachee".to_string()),
                speaker_label: Some("Speaker B".to_string()),
            },
        ],
        resource_types: vec!["coaching_action".to_string()],
        generate_summary: true,
        extraction_prompt: r#"
            Extract coaching_action items from this coaching session transcript.
            For each action:
            - content: What needs to be done (required)
            - assigned_to: Name of person responsible (required)
            - due_by: When it should be completed (optional, ISO 8601 format)
            - confidence: 0.0-1.0 score for extraction confidence
            - timestamp_ms: When action was mentioned in meeting
            - source_text: Exact quote from transcript

            Only extract clear commitments and tasks, not general discussion.
        "#.to_string(),
        model: Some("gpt-4".to_string()),
        provider_options: HashMap::new(),
    };

    // Get analysis results
    let result = provider.analyze(config).await?;

    // Type-safe extraction of application-specific resources
    let coaching_actions: Vec<CoachingAction> = result.get_resources()?;

    // Persist to database using SeaORM
    for action in coaching_actions {
        let active_model: actions::ActiveModel = action.into();
        active_model.insert(&db).await?;
    }

    Ok(Json(result))
}
```

This pattern works for any domain - medical consultations extracting diagnoses and prescriptions,
sales calls extracting leads and objections, project meetings extracting tasks and risks, etc.

### 6. Webhook Event System

```rust
/// Unified webhook event types from all providers (bots, transcription, analysis).
/// Events enable real-time UI updates and workflow automation without polling.
/// Applications implement WebhookHandler to process these normalized events.
/// The type tag enables type-safe deserialization and event routing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebhookEvent {
    /// Bot transitioned between lifecycle states (Pending → Joining → Recording → etc.).
    /// Monitor this for UI status indicators and workflow progression triggers.
    /// Check error_message when new_status is Failed to diagnose issues.
    BotStatusChanged {
        bot_id: String,
        old_status: BotStatus,
        new_status: BotStatus,
        timestamp: DateTime<Utc>,
        error_message: Option<String>,
    },

    /// Bot finished recording and artifacts (video/audio URLs) are ready.
    /// Trigger transcription immediately as artifact URLs expire in 24-48 hours.
    /// Download and persist files or pass URLs directly to transcription provider.
    BotRecordingCompleted {
        bot_id: String,
        artifacts: RecordingArtifacts,
        timestamp: DateTime<Utc>,
    },

    /// Transcription job status changed (Queued → Processing → Completed/Failed).
    /// Use this to update UI progress indicators without polling.
    /// Check error_message when new_status is Failed for debugging.
    TranscriptionStatusChanged {
        transcription_id: String,
        old_status: TranscriptionStatus,
        new_status: TranscriptionStatus,
        timestamp: DateTime<Utc>,
        error_message: Option<String>,
    },

    /// Transcription finished and full transcript with enhancements is available.
    /// Trigger AI analysis immediately or update database with transcript results.
    /// All fields (words, segments, chapters) are populated in transcript.
    TranscriptionCompleted {
        transcription_id: String,
        transcript: Transcription,
        timestamp: DateTime<Utc>,
    },

    /// Real-time transcript segment from streaming transcription during live meeting.
    /// Use for live captions, real-time search, or interim meeting notes.
    /// is_final=true means segment won't change; false means preliminary result.
    TranscriptData {
        bot_id: String,
        text: String,
        speaker: Option<String>,
        is_final: bool,
        timestamp: DateTime<Utc>,
    },
}

/// Application-level webhook event handler for processing provider callbacks.
/// Implementations update database state, trigger downstream workflows,
/// and send real-time updates to connected clients via WebSockets/SSE.
/// Handlers must be idempotent as providers may retry delivery.
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    /// Process incoming webhook event and update application state.
    /// Should be idempotent as providers retry failed deliveries.
    /// Return Ok(()) to acknowledge receipt; Err triggers provider retry.
    async fn handle_event(&self, event: WebhookEvent) -> Result<(), Error>;
}
```

### 7. Workflow Orchestrator (Optional)

```rust
/// High-level orchestrator that coordinates end-to-end meeting AI workflow.
/// Composes provider traits to automate: bot recording → transcription → AI analysis.
/// Simplifies application code by handling complex multi-step async workflows
/// with error recovery and state persistence.
pub struct MeetingWorkflow {
    pub bot_provider: Box<dyn RecordingBotProvider>,
    pub transcription_provider: Box<dyn TranscriptionProvider>,
    pub analysis_provider: Box<dyn AnalysisProvider>,
}

/// State machine representing workflow progression through meeting AI pipeline.
/// Enables UI progress indicators, workflow resumption, and error recovery.
/// Failed state may occur at any step; check WorkflowProgress.error for details.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowState {
    NotStarted,
    BotJoining,
    Recording,
    ProcessingRecording,
    Transcribing,
    Analyzing,
    Completed,
    Failed,
}

/// Complete workflow state for persistence and resumption.
/// Applications serialize this to database to support workflow recovery after restarts.
/// Fields populate progressively as workflow advances through states.
/// Monitor updated_at to detect stalled workflows requiring intervention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProgress {
    pub state: WorkflowState,
    pub bot_info: Option<BotInfo>,
    pub transcription: Option<Transcription>,
    pub analysis: Option<AnalysisResult>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MeetingWorkflow {
    /// Initiate AI workflow for an existing meeting.
    /// Orchestrates bot deployment, transcription, and AI analysis with error handling.
    /// Returns progress object; use webhooks or resume_workflow to complete async operations.
    /// Applications should persist returned progress for workflow recovery.
    pub async fn start_workflow(
        &self,
        bot_config: BotConfig,
        transcription_config: TranscriptionConfig,
        analysis_config: AnalysisConfig,
    ) -> Result<WorkflowProgress, Error> {
        // Orchestrate the entire flow
        todo!()
    }

    /// Resume workflow from saved state after system restart or async operations.
    /// Checks current state and progresses workflow to next step.
    /// Use this in webhook handlers or cron jobs to drive long-running workflows.
    /// Returns updated progress; repeat until state is Completed or Failed.
    pub async fn resume_workflow(&self, progress: WorkflowProgress) -> Result<WorkflowProgress, Error> {
        todo!()
    }
}
```

## Implementation Strategy

### Phase 1: Create Standalone Crate

```toml
# meeting-ai/Cargo.toml
[package]
name = "meeting-ai"
version = "0.1.0"
edition = "2021"

[dependencies]
meeting-auth = { path = "../meeting-auth" }
meeting-manager = { path = "../meeting-manager" }
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
```

Directory structure:
```
meeting-ai/
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── traits/
│   │   ├── mod.rs
│   │   ├── recording_bot.rs
│   │   ├── transcription.rs
│   │   ├── ai_analysis.rs
│   │   └── webhook.rs
│   ├── types/
│   │   ├── mod.rs
│   │   ├── recording.rs
│   │   ├── transcription.rs
│   │   └── analysis.rs
│   └── workflow.rs
├── examples/
│   ├── basic_workflow.rs
│   └── custom_providers.rs
└── tests/
    └── integration_tests.rs
```

### Phase 2: Refactor Existing Implementations

Convert existing code to implement the traits:

```rust
// domain/src/gateway/recall_ai_provider.rs
use meeting_ai::{RecordingBotProvider, BotConfig, BotInfo, Error};
use meeting_auth::{ProviderAuth, ApiKeyAuth, AuthenticatedClientBuilder};

pub struct RecallAiProvider {
    client: reqwest_middleware::ClientWithMiddleware,
    base_url: String,
}

impl RecallAiProvider {
    pub fn new(api_key: String, region: &str) -> Result<Self, Error> {
        let auth = Box::new(ApiKeyAuth::new("recall_ai", api_key, "Token"));
        let client = AuthenticatedClientBuilder::new()
            .with_auth(auth)
            .with_retry_policy(RetryAfterPolicy::new(3))
            .build()?;

        Ok(Self {
            client,
            base_url: format!("https://api.recall.ai/{}", region),
        })
    }
}

#[async_trait]
impl RecordingBotProvider for RecallAiProvider {
    async fn create_bot(&self, config: BotConfig) -> Result<BotInfo, Error> {
        // Map from trait types to Recall.ai types
        let request = create_standard_bot_request(
            config.meeting_url,
            config.bot_name,
            config.webhook_url,
        );

        let response = self.client
            .post(&format!("{}/bots", self.base_url))
            .json(&request)
            .send()
            .await?
            .json::<CreateBotResponse>()
            .await?;

        // Map from Recall.ai types back to trait types
        Ok(BotInfo {
            id: response.id,
            meeting_url: config.meeting_url,
            status: map_recall_status(&response.status_changes),
            artifacts: None,
            error_message: None,
            status_history: vec![],
        })
    }

    // ... implement other trait methods
}
```

### Phase 3: Update Controllers

Simplify controllers by working with traits instead of concrete types:

```rust
// web/src/controller/meeting_recording_controller.rs
pub async fn start_recording(
    State(app_state): State<AppState>,
    Path(session_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    // Get the configured provider (could be Recall.ai, Skribby, etc.)
    let bot_provider = app_state.get_bot_provider(user.id).await?;

    let bot_config = BotConfig {
        meeting_url,
        bot_name: "Refactor Coaching Notetaker".to_string(),
        webhook_url: config.webhook_base_url().map(|b| format!("{}/webhooks/bot", b)),
        record_video: true,
        record_audio: true,
        enable_realtime_transcription: true,
        provider_options: HashMap::new(),
    };

    // Now provider-agnostic!
    let bot_info = bot_provider.create_bot(bot_config).await?;

    // Store bot info in database
    // ...
}
```

### Phase 4: Add Alternative Providers

Implement new providers without touching existing code:

```rust
// domain/src/gateway/skribby_provider.rs
use meeting_ai::{RecordingBotProvider, BotConfig, BotInfo, Error};
use meeting_auth::{ProviderAuth, ApiKeyAuth, AuthenticatedClientBuilder};

pub struct SkribbyProvider {
    client: reqwest_middleware::ClientWithMiddleware,
}

impl SkribbyProvider {
    pub fn new(api_key: String) -> Result<Self, Error> {
        let auth = Box::new(ApiKeyAuth::new("skribby", api_key, "Bearer"));
        let client = AuthenticatedClientBuilder::new()
            .with_auth(auth)
            .build()?;

        Ok(Self { client })
    }
}

#[async_trait]
impl RecordingBotProvider for SkribbyProvider {
    async fn create_bot(&self, config: BotConfig) -> Result<BotInfo, Error> {
        // Skribby-specific implementation
        todo!()
    }

    // ... implement other trait methods
}
```

### Phase 5: Testing

Mock providers for unit tests:

```rust
#[cfg(test)]
mod tests {
    use mockall::mock;

    mock! {
        pub BotProvider {}

        #[async_trait]
        impl RecordingBotProvider for BotProvider {
            async fn create_bot(&self, config: BotConfig) -> Result<BotInfo, Error>;
            async fn get_bot_status(&self, bot_id: &str) -> Result<BotInfo, Error>;
            async fn stop_bot(&self, bot_id: &str) -> Result<(), Error>;
            async fn list_bots(&self, filters: Option<BotFilters>) -> Result<Vec<BotInfo>, Error>;
            fn provider_id(&self) -> &str;
            async fn verify_credentials(&self) -> Result<bool, Error>;
        }
    }

    #[tokio::test]
    async fn test_start_recording() {
        let mut mock_provider = MockBotProvider::new();
        mock_provider
            .expect_create_bot()
            .returning(|_| Ok(BotInfo { /* ... */ }));

        // Test your controller with the mock
    }
}
```

## Migration Path

### Step 1: Non-Breaking Addition
1. Create `meeting-ai` crate alongside existing code
2. Keep existing implementations working as-is
3. Add new trait implementations that wrap existing clients

### Step 2: Gradual Refactor
1. Update one controller at a time to use traits
2. Add feature flag for new provider system
3. Run both systems in parallel during transition

### Step 3: Complete Migration
1. Remove direct dependencies on provider types from controllers
2. Move provider selection logic to configuration
3. Enable hot-swapping providers via config

### Step 4: Deprecation
1. Mark old implementations as deprecated
2. Provide migration guide
3. Remove old code in next major version

## Benefits

### For Developers
- **Cleaner code** - Work with interfaces, not implementations
- **Easier testing** - Mock providers for unit tests
- **Better IDE support** - Clear trait boundaries

### For Operations
- **Provider flexibility** - Swap providers without code changes
- **Cost optimization** - Choose cheapest provider per use case
- **Vendor independence** - No lock-in to specific providers

### For Business
- **Faster feature development** - Reusable abstractions
- **Risk mitigation** - Not dependent on single provider
- **Future-proof** - Easy to adopt new AI technologies

## Open Questions

1. **Provider Discovery** - How should applications discover and select providers at runtime?
   - Configuration-based?
   - Service registry pattern?
   - Factory pattern?

2. **State Persistence** - Should the abstraction layer handle state persistence or leave it to applications?
   - Propose: Leave to applications (use existing entity models)

3. **Retry Logic** - Should providers implement retry logic internally?
   - Propose: Use `tower` middleware for retry/circuit breaker

4. **Rate Limiting** - How to handle provider-specific rate limits?
   - Propose: Return `Error::RateLimited` with retry-after info

5. **Multi-Provider** - Should workflows support multiple providers simultaneously?
   - Example: Record with Recall.ai, transcribe with AssemblyAI AND Deepgram for comparison
   - Propose: Yes, via aggregator pattern

6. **Versioning** - How to handle provider API version changes?
   - Propose: Provider-specific options HashMap for version-specific features

## Next Steps

1. **Review this design** with team for feedback
2. **Create PoC** - Implement one trait + one provider as proof of concept
3. **Define interfaces** - Finalize trait signatures with input from stakeholders
4. **Implement crate** - Build out the standalone `meeting-ai` crate
5. **Refactor one workflow** - Migrate recording OR transcription as pilot
6. **Document patterns** - Create guides for adding new providers
7. **Publish to crates.io** - Make available for community use

## References

**Related Plans:**
- [Meeting Auth & Meeting Manager Crates](./meeting-and-auth-abstraction-layers.md) - OAuth, meeting creation, HTTP clients, webhooks

**External Resources:**
- [Rust async abstraction patterns](https://ewus.de/en/blog/2022-11-06/rust-async-abstraction-pattern)
- [Type-driven API design in Rust](https://willcrichton.net/rust-api-type-patterns/)
- [Recall.ai API Documentation](https://www.recall.ai/)
- [AssemblyAI API Documentation](https://www.assemblyai.com/docs)
- [Meeting transcription workflow best practices](https://superagi.com/2025-trends-in-ai-meeting-transcription-whats-new-and-whats-next-for-remote-teams/)

---

**End of Design Document**
