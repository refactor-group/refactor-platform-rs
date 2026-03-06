//! Credential storage trait for API keys.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Error;

/// Credential data for storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Data {
    /// API key or secret.
    pub api_key: String,
    /// Optional region (e.g., for Recall.ai).
    pub region: Option<String>,
    /// Optional base URL override.
    pub base_url: Option<String>,
    /// Provider-specific configuration.
    pub config: serde_json::Value,
}

/// Trait for storing provider credentials (API keys).
///
/// Implementations should:
/// - Encrypt credentials at rest (e.g., using AES-256-GCM)
/// - Handle concurrent access safely
#[async_trait]
pub trait Storage: Send + Sync {
    /// Store credentials for a user and provider.
    async fn store(&self, user_id: &str, provider_id: &str, credentials: Data)
        -> Result<(), Error>;

    /// Retrieve credentials for a user and provider.
    async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<Data>, Error>;

    /// Update credentials for a user and provider.
    async fn update(
        &self,
        user_id: &str,
        provider_id: &str,
        credentials: Data,
    ) -> Result<(), Error>;

    /// Delete credentials for a user and provider.
    async fn delete(&self, user_id: &str, provider_id: &str) -> Result<(), Error>;
}
