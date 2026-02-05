//! Token storage trait for persisting OAuth tokens.

use async_trait::async_trait;

use super::Tokens;
use crate::error::Error;

/// Trait for storing and retrieving OAuth tokens.
///
/// CRITICAL: Implementations must support atomic updates for Zoom's rotating refresh tokens.
/// The `update_atomic` method ensures that concurrent refresh attempts don't cause race conditions.
///
/// Implementations should:
/// - Encrypt tokens at rest (e.g., using AES-256-GCM)
/// - Use database transactions for atomic updates
/// - Handle concurrent access safely
#[async_trait]
pub trait Storage: Send + Sync {
    /// Store tokens for a user and provider.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier (e.g., "google", "zoom")
    /// * `tokens` - The tokens to store
    async fn store(&self, user_id: &str, provider_id: &str, tokens: Tokens) -> Result<(), Error>;

    /// Retrieve tokens for a user and provider.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    ///
    /// # Returns
    ///
    /// `Some(Tokens)` if found, `None` if not found.
    async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<Tokens>, Error>;

    /// Atomically update tokens if the old refresh token matches.
    ///
    /// CRITICAL for Zoom's rotating refresh tokens. This method ensures that only one
    /// concurrent refresh succeeds when multiple requests try to refresh simultaneously.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    /// * `old_refresh` - Expected current refresh token (for compare-and-swap)
    /// * `new_tokens` - The new tokens to store
    ///
    /// # Returns
    ///
    /// `Ok(())` if update succeeded, `Err` if old_refresh doesn't match (token already rotated).
    async fn update_atomic(
        &self,
        user_id: &str,
        provider_id: &str,
        old_refresh: Option<&str>,
        new_tokens: Tokens,
    ) -> Result<(), Error>;

    /// Delete tokens for a user and provider.
    ///
    /// # Arguments
    ///
    /// * `user_id` - Unique user identifier
    /// * `provider_id` - Provider identifier
    async fn delete(&self, user_id: &str, provider_id: &str) -> Result<(), Error>;
}
