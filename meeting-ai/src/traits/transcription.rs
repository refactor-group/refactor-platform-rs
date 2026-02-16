//! Transcription provider trait.

use crate::types::transcription::{Config, Transcription};
use crate::Error;
use async_trait::async_trait;

/// Abstraction for speech-to-text transcription services.
///
/// Implementations convert audio/video to text with speaker diarization, timing,
/// and optional enhancements (sentiment, chapters). Supports AssemblyAI, Deepgram, Whisper.
/// This trait enables provider swapping for cost optimization and feature comparison.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Start async transcription job for audio/video at media_url.
    ///
    /// Returns immediately with job ID; results available via get_transcription when complete.
    /// Media must be publicly accessible or use pre-signed URL with sufficient expiry.
    async fn create_transcription(
        &self,
        config: Config,
    ) -> std::result::Result<Transcription, Error>;

    /// Retrieve transcription status and results by ID.
    ///
    /// Poll until status is Completed or Failed. Rate limit polling to avoid quota waste.
    /// All fields (words, segments, etc.) populate only when status is Completed.
    async fn get_transcription(
        &self,
        transcription_id: &str,
    ) -> std::result::Result<Transcription, Error>;

    /// Permanently delete transcription and associated data from provider storage.
    ///
    /// Use for GDPR compliance, data retention policies, or cleaning up test data.
    /// Some providers auto-delete after retention period (e.g., 30 days).
    async fn delete_transcription(&self, transcription_id: &str) -> std::result::Result<(), Error>;

    /// Return unique identifier for this provider (e.g., "assemblyai", "deepgram").
    ///
    /// Used for cost tracking, feature-specific logic, and provider selection.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    ///
    /// Call during user onboarding or settings updates for immediate validation feedback.
    /// Returns false if credentials invalid, expired, or lack transcription permissions.
    async fn verify_credentials(&self) -> std::result::Result<bool, Error>;
}
