//! Recording bot provider trait.

use crate::types::recording::{Config, Filters, Info};
use crate::Error;
use async_trait::async_trait;

/// Abstraction for meeting bot services that join meetings to record.
///
/// Implementations deploy virtual participants to meetings, record video/audio,
/// and return media artifacts. Supports providers like Recall.ai, Skribby, Meeting BaaS.
/// This trait enables cost optimization by swapping providers without code changes.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Deploy a bot to join and record a meeting.
    ///
    /// Bot immediately begins joining process; track progress via webhooks or polling.
    /// Returns Info with id for subsequent status checks and bot control.
    async fn create_bot(&self, config: Config) -> std::result::Result<Info, Error>;

    /// Retrieve current status and available artifacts for a bot.
    ///
    /// Poll this endpoint if webhook_url was not configured during creation.
    /// Artifacts populate when status reaches Completed or Processing.
    async fn get_bot_status(&self, bot_id: &str) -> std::result::Result<Info, Error>;

    /// Immediately remove bot from meeting and stop recording.
    ///
    /// Use when user manually ends recording early or cancels session.
    /// Partial recordings may still be available depending on provider.
    async fn stop_bot(&self, bot_id: &str) -> std::result::Result<(), Error>;

    /// Query all bots with optional filters (status, meeting URL, date range).
    ///
    /// Useful for admin dashboards, debugging, or finding bots by meeting.
    /// Large result sets may require pagination (implement in provider_options).
    async fn list_bots(&self, filters: Option<Filters>) -> std::result::Result<Vec<Info>, Error>;

    /// Return unique identifier for this provider (e.g., "recall_ai", "skribby").
    ///
    /// Used for logging, cost tracking, and selecting providers at runtime.
    /// Must be lowercase, alphanumeric with underscores only.
    fn provider_id(&self) -> &str;

    /// Validate API credentials by making a lightweight test request.
    ///
    /// Call during user onboarding or settings updates to provide immediate feedback.
    /// Returns false if credentials are invalid, expired, or lack permissions.
    async fn verify_credentials(&self) -> std::result::Result<bool, Error>;
}
