# Meeting AI SDK - Abstraction Layer Design

**Status:** Design Phase
**Date:** 2025-01-28
**Author:** Architecture Review
**Approach:** Hybrid (Standalone crate + Domain implementations)

## Executive Summary

This document proposes a comprehensive abstraction layer for meeting AI workflows that decouples business logic from specific third-party providers (Recall.ai, AssemblyAI, Google OAuth, etc.). The design enables applications to build meeting recording, transcription, and AI analysis workflows without being locked into specific vendors.

## Current State Analysis

### Existing Workflow

The current implementation in `refactor-platform-rs` follows this flow:

1. **OAuth Authentication** (Google OAuth) → Get access token for meeting creation
2. **Meeting Creation** (Google Meet API) → Create meeting space, get URL
3. **Recording** (Recall.ai) → Deploy bot to join and record meeting
4. **Transcription** (AssemblyAI) → Transcribe recording with speaker diarization
5. **AI Analysis** (AssemblyAI LeMUR) → Extract actions, agreements, generate summaries
6. **Webhook Processing** → Handle async status updates from providers

### Key Files

- `domain/src/gateway/recall_ai.rs` - Recall.ai bot management
- `domain/src/gateway/assembly_ai.rs` - Transcription and LeMUR analysis
- `domain/src/gateway/google_oauth.rs` - OAuth and Meet API
- `web/src/controller/meeting_recording_controller.rs` - Recording endpoints
- `web/src/controller/transcription_controller.rs` - Transcription endpoints
- `web/src/controller/webhook_controller.rs` - Webhook handlers

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

**Standalone Crate** (`meeting-ai-sdk`)
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

```
MeetingPlatformProvider     (OAuth, meeting creation)
    ├── GoogleMeetProvider
    ├── ZoomProvider
    └── TeamsProvider

MeetingBotProvider          (Meeting bots)
    ├── RecallAiProvider
    ├── SkribbyProvider
    └── MeetingBaasProvider

TranscriptionProvider       (Speech-to-text)
    ├── AssemblyAiProvider
    ├── DeepgramProvider
    └── WhisperProvider

AiAnalysisProvider         (Action extraction, summaries)
    ├── LemurProvider
    ├── OpenAiProvider
    └── ClaudeProvider

WebhookHandler             (Event processing)
    └── Custom implementations per app
```

## Complete Trait Definitions

### 1. Core Types

```rust
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

/// Universal error type for the SDK
#[derive(Debug, Error)]
pub enum SdkError {
    #[error("Authentication failed: {0}")]
    Authentication(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Invalid configuration: {0}")]
    Configuration(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Rate limited: retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },

    #[error("Other error: {0}")]
    Other(Box<dyn std::error::Error + Send + Sync>),
}

pub type SdkResult<T> = Result<T, SdkError>;
```

### 2. Meeting Platform Provider

```rust
/// Configuration for OAuth authentication
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
}

/// OAuth tokens returned from authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub token_type: String,
}

/// User information from the platform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformUser {
    pub id: String,
    pub email: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

/// A meeting space/room created on the platform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSpace {
    pub id: String,
    pub meeting_url: String,
    pub meeting_code: Option<String>,
    pub platform: String,
    pub metadata: HashMap<String, String>,
}

/// Configuration for creating a meeting space
#[derive(Debug, Clone, Default)]
pub struct MeetingSpaceConfig {
    pub title: Option<String>,
    pub description: Option<String>,
    pub start_time: Option<DateTime<Utc>>,
    pub duration_minutes: Option<u32>,
    pub is_public: bool,
}

/// Trait for meeting platform providers (Google Meet, Zoom, Teams, etc.)
#[async_trait]
pub trait MeetingPlatformProvider: Send + Sync {
    /// Get the authorization URL for OAuth flow
    fn get_authorization_url(&self, state: &str) -> SdkResult<String>;

    /// Exchange authorization code for access tokens
    async fn exchange_code(&self, code: &str) -> SdkResult<OAuthTokens>;

    /// Refresh an expired access token
    async fn refresh_token(&self, refresh_token: &str) -> SdkResult<OAuthTokens>;

    /// Get user information using an access token
    async fn get_user_info(&self, access_token: &str) -> SdkResult<PlatformUser>;

    /// Verify if an access token is still valid
    async fn verify_token(&self, access_token: &str) -> SdkResult<bool>;

    /// Create a new meeting space
    async fn create_meeting_space(
        &self,
        access_token: &str,
        config: Option<MeetingSpaceConfig>
    ) -> SdkResult<MeetingSpace>;

    /// Get the platform identifier (e.g., "google_meet", "zoom")
    fn platform_id(&self) -> &str;
}
```

### 3. Recording Bot Provider

```rust
/// Status of a recording bot
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

/// Recording artifacts (video, audio, etc.)
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

/// Information about a recording bot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotInfo {
    pub id: String,
    pub meeting_url: String,
    pub status: BotStatus,
    pub artifacts: Option<RecordingArtifacts>,
    pub error_message: Option<String>,
    pub status_history: Vec<BotStatusChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotStatusChange {
    pub status: BotStatus,
    pub timestamp: DateTime<Utc>,
    pub message: Option<String>,
}

/// Configuration for creating a recording bot
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

#[derive(Debug, Clone, Default)]
pub struct BotFilters {
    pub status: Option<BotStatus>,
    pub meeting_url: Option<String>,
    pub created_after: Option<DateTime<Utc>>,
}

/// Trait for meeting bot providers (Recall.ai, Skribby, etc.)
#[async_trait]
pub trait MeetingBotProvider: Send + Sync {
    /// Create a new recording bot to join a meeting
    async fn create_bot(&self, config: BotConfig) -> SdkResult<BotInfo>;

    /// Get the current status of a bot
    async fn get_bot_status(&self, bot_id: &str) -> SdkResult<BotInfo>;

    /// Stop/remove a bot from the meeting
    async fn stop_bot(&self, bot_id: &str) -> SdkResult<()>;

    /// List all bots (with optional filters)
    async fn list_bots(&self, filters: Option<BotFilters>) -> SdkResult<Vec<BotInfo>>;

    /// Get the provider identifier (e.g., "recall_ai", "skribby")
    fn provider_id(&self) -> &str;

    /// Verify API credentials are valid
    async fn verify_credentials(&self) -> SdkResult<bool>;
}
```

### 4. Transcription Provider

```rust
/// Status of a transcription job
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TranscriptionStatus {
    Queued,
    Processing,
    Completed,
    Failed,
}

/// A word in the transcript with timing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptWord {
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub speaker: Option<String>,
}

/// A speaker segment (utterance) in the transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptSegment {
    pub text: String,
    pub speaker: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub confidence: f64,
    pub words: Vec<TranscriptWord>,
}

/// A chapter/section auto-detected in the transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChapter {
    pub title: String,
    pub summary: String,
    pub gist: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// Sentiment analysis result
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sentiment {
    Positive,
    Neutral,
    Negative,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SentimentAnalysis {
    pub text: String,
    pub sentiment: Sentiment,
    pub confidence: f64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub speaker: Option<String>,
}

/// Complete transcription result
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

/// Configuration for creating a transcription
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

/// Trait for transcription providers (AssemblyAI, Deepgram, etc.)
#[async_trait]
pub trait TranscriptionProvider: Send + Sync {
    /// Create a new transcription job
    async fn create_transcription(&self, config: TranscriptionConfig) -> SdkResult<Transcription>;

    /// Get the status/results of a transcription
    async fn get_transcription(&self, transcription_id: &str) -> SdkResult<Transcription>;

    /// Delete a transcription and its data
    async fn delete_transcription(&self, transcription_id: &str) -> SdkResult<()>;

    /// Get the provider identifier (e.g., "assemblyai", "deepgram")
    fn provider_id(&self) -> &str;

    /// Verify API credentials are valid
    async fn verify_credentials(&self) -> SdkResult<bool>;
}
```

### 5. AI Analysis Provider

```rust
/// An action item extracted from a transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedAction {
    pub content: String,
    pub source_text: String,
    pub stated_by: String,
    pub assigned_to: String,
    pub assigned_to_name: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    pub confidence: f64,
    pub timestamp_ms: Option<i64>,
}

/// An agreement/commitment extracted from a transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedAgreement {
    pub content: String,
    pub source_text: String,
    pub stated_by: String,
    pub confidence: f64,
    pub timestamp_ms: Option<i64>,
}

/// Key topics/themes identified in the transcript
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedTopic {
    pub name: String,
    pub relevance_score: f64,
    pub mentions: Vec<TopicMention>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicMention {
    pub text: String,
    pub timestamp_ms: i64,
}

/// Structured summary of a meeting/conversation
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

/// Result from AI analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub request_id: String,
    pub actions: Vec<ExtractedAction>,
    pub agreements: Vec<ExtractedAgreement>,
    pub summary: Option<MeetingSummary>,
    pub token_usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Configuration for AI analysis
#[derive(Debug, Clone)]
pub struct AnalysisConfig {
    pub transcript_id: String,
    pub participants: Vec<Participant>,
    pub extract_actions: bool,
    pub extract_agreements: bool,
    pub generate_summary: bool,
    pub custom_prompt: Option<String>,
    pub model: Option<String>,
    pub provider_options: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Participant {
    pub name: String,
    pub role: Option<String>,
    pub speaker_label: Option<String>,
}

/// Trait for AI analysis providers (AssemblyAI LeMUR, OpenAI, etc.)
#[async_trait]
pub trait AiAnalysisProvider: Send + Sync {
    /// Analyze a transcript and extract insights
    async fn analyze(&self, config: AnalysisConfig) -> SdkResult<AnalysisResult>;

    /// Execute a custom analysis prompt on a transcript
    async fn custom_task(&self, transcript_id: &str, prompt: &str) -> SdkResult<String>;

    /// Get the provider identifier (e.g., "lemur", "openai")
    fn provider_id(&self) -> &str;

    /// Verify API credentials are valid
    async fn verify_credentials(&self) -> SdkResult<bool>;
}
```

### 6. Webhook Event System

```rust
/// Type of webhook event
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WebhookEvent {
    BotStatusChanged {
        bot_id: String,
        old_status: BotStatus,
        new_status: BotStatus,
        timestamp: DateTime<Utc>,
        error_message: Option<String>,
    },
    BotRecordingCompleted {
        bot_id: String,
        artifacts: RecordingArtifacts,
        timestamp: DateTime<Utc>,
    },
    TranscriptionStatusChanged {
        transcription_id: String,
        old_status: TranscriptionStatus,
        new_status: TranscriptionStatus,
        timestamp: DateTime<Utc>,
        error_message: Option<String>,
    },
    TranscriptionCompleted {
        transcription_id: String,
        transcript: Transcription,
        timestamp: DateTime<Utc>,
    },
    TranscriptData {
        bot_id: String,
        text: String,
        speaker: Option<String>,
        is_final: bool,
        timestamp: DateTime<Utc>,
    },
}

/// Trait for handling webhook events
#[async_trait]
pub trait WebhookHandler: Send + Sync {
    /// Handle an incoming webhook event
    async fn handle_event(&self, event: WebhookEvent) -> SdkResult<()>;

    /// Verify webhook authenticity (signature, secret, etc.)
    fn verify_webhook(&self, headers: &HashMap<String, String>, body: &[u8]) -> SdkResult<bool>;
}
```

### 7. Workflow Orchestrator (Optional)

```rust
/// High-level orchestrator that coordinates the entire workflow
pub struct MeetingWorkflow {
    pub meeting_provider: Box<dyn MeetingPlatformProvider>,
    pub bot_provider: Box<dyn MeetingBotProvider>,
    pub transcription_provider: Box<dyn TranscriptionProvider>,
    pub analysis_provider: Box<dyn AiAnalysisProvider>,
}

/// Workflow state machine
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkflowState {
    NotStarted,
    MeetingCreated,
    BotJoining,
    Recording,
    ProcessingRecording,
    Transcribing,
    Analyzing,
    Completed,
    Failed,
}

/// Progress information for a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProgress {
    pub state: WorkflowState,
    pub meeting_space: Option<MeetingSpace>,
    pub bot_info: Option<BotInfo>,
    pub transcription: Option<Transcription>,
    pub analysis: Option<AnalysisResult>,
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl MeetingWorkflow {
    /// Start a complete meeting workflow
    pub async fn start_workflow(
        &self,
        access_token: &str,
        bot_config: BotConfig,
        transcription_config: TranscriptionConfig,
        analysis_config: AnalysisConfig,
    ) -> SdkResult<WorkflowProgress> {
        // Orchestrate the entire flow
        todo!()
    }

    /// Resume a workflow from saved state
    pub async fn resume_workflow(&self, progress: WorkflowProgress) -> SdkResult<WorkflowProgress> {
        todo!()
    }
}
```

## Implementation Strategy

### Phase 1: Create Standalone Crate

```toml
# meeting-ai-sdk/Cargo.toml
[package]
name = "meeting-ai-sdk"
version = "0.1.0"
edition = "2021"

[dependencies]
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "1.0"
```

Directory structure:
```
meeting-ai-sdk/
├── src/
│   ├── lib.rs
│   ├── error.rs
│   ├── traits/
│   │   ├── mod.rs
│   │   ├── meeting_platform.rs
│   │   ├── recording_bot.rs
│   │   ├── transcription.rs
│   │   ├── ai_analysis.rs
│   │   └── webhook.rs
│   ├── types/
│   │   ├── mod.rs
│   │   ├── meeting.rs
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
use meeting_ai_sdk::{MeetingBotProvider, BotConfig, BotInfo, SdkResult};

pub struct RecallAiProvider {
    client: RecallAiClient,
}

#[async_trait]
impl MeetingBotProvider for RecallAiProvider {
    async fn create_bot(&self, config: BotConfig) -> SdkResult<BotInfo> {
        // Map from SDK types to Recall.ai types
        let request = create_standard_bot_request(
            config.meeting_url,
            config.bot_name,
            config.webhook_url,
        );

        let response = self.client.create_bot(request).await?;

        // Map from Recall.ai types back to SDK types
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
pub struct SkribbyProvider {
    api_key: String,
    client: reqwest::Client,
}

#[async_trait]
impl MeetingBotProvider for SkribbyProvider {
    async fn create_bot(&self, config: BotConfig) -> SdkResult<BotInfo> {
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
        impl MeetingBotProvider for BotProvider {
            async fn create_bot(&self, config: BotConfig) -> SdkResult<BotInfo>;
            async fn get_bot_status(&self, bot_id: &str) -> SdkResult<BotInfo>;
            async fn stop_bot(&self, bot_id: &str) -> SdkResult<()>;
            async fn list_bots(&self, filters: Option<BotFilters>) -> SdkResult<Vec<BotInfo>>;
            fn provider_id(&self) -> &str;
            async fn verify_credentials(&self) -> SdkResult<bool>;
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
1. Create `meeting-ai-sdk` crate alongside existing code
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

2. **State Persistence** - Should the SDK handle state persistence or leave it to applications?
   - Propose: Leave to applications (use existing entity models)

3. **Retry Logic** - Should providers implement retry logic internally?
   - Propose: Use `tower` middleware for retry/circuit breaker

4. **Rate Limiting** - How to handle provider-specific rate limits?
   - Propose: Return `SdkError::RateLimited` with retry-after info

5. **Multi-Provider** - Should workflows support multiple providers simultaneously?
   - Example: Record with Recall.ai, transcribe with AssemblyAI AND Deepgram for comparison
   - Propose: Yes, via aggregator pattern

6. **Versioning** - How to handle provider API version changes?
   - Propose: Provider-specific options HashMap for version-specific features

## Next Steps

1. **Review this design** with team for feedback
2. **Create PoC** - Implement one trait + one provider as proof of concept
3. **Define interfaces** - Finalize trait signatures with input from stakeholders
4. **Implement SDK crate** - Build out the standalone crate
5. **Refactor one workflow** - Migrate recording OR transcription as pilot
6. **Document patterns** - Create guides for adding new providers
7. **Publish to crates.io** - Make available for community use

## References

- [Rust async abstraction patterns](https://ewus.de/en/blog/2022-11-06/rust-async-abstraction-pattern)
- [Type-driven API design in Rust](https://willcrichton.net/rust-api-type-patterns/)
- [Recall.ai API Documentation](https://www.recall.ai/)
- [AssemblyAI API Documentation](https://www.assemblyai.com/docs)
- [Meeting transcription workflow best practices](https://superagi.com/2025-trends-in-ai-meeting-transcription-whats-new-and-whats-next-for-remote-teams/)

---

**End of Design Document**
