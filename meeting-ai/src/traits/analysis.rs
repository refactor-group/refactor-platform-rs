//! AI analysis provider trait.

use crate::types::analysis::{Config, Result};
use crate::Error;
use async_trait::async_trait;

/// Abstraction for LLM-powered meeting transcript analysis.
///
/// Implementations use large language models to extract domain-specific resources
/// from transcripts. Supports AssemblyAI LeMUR, OpenAI GPT-4, Anthropic Claude.
/// This trait enables model comparison, cost optimization, and provider switching.
/// Completely domain-agnostic - applications define what to extract via Config.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Analyze transcript and extract structured resources based on config.
    ///
    /// Processing typically takes 10-60 seconds depending on transcript length and model.
    /// Returns domain-specific resources defined by application's extraction_prompt.
    /// Resources organized by type in Result for type-safe deserialization.
    async fn analyze(&self, config: Config) -> std::result::Result<Result, Error>;

    /// Run custom LLM prompt against transcript for domain-specific analysis.
    ///
    /// Use for specialized extractions not covered by standard analyze() method.
    /// Returns raw LLM response; parse result according to your prompt instructions.
    async fn custom_task(
        &self,
        transcript_id: &str,
        prompt: &str,
    ) -> std::result::Result<String, Error>;

    /// Return unique identifier for this provider (e.g., "lemur", "openai", "claude").
    ///
    /// Used for cost tracking, model-specific logic, and provider selection.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    ///
    /// Call during user onboarding or settings updates for immediate validation.
    /// Returns false if credentials invalid, expired, or lack analysis permissions.
    async fn verify_credentials(&self) -> std::result::Result<bool, Error>;
}
