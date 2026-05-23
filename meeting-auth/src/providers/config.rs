//! Pre-configured provider settings.

use crate::api_key::Provider as ApiKeyProvider;

/// Provider configuration with endpoints and settings.
#[derive(Debug, Clone)]
pub struct Config {
    /// Provider identifier.
    pub provider: ApiKeyProvider,
    /// Base API URL.
    pub base_url: String,
    /// Default region (if applicable).
    pub region: Option<String>,
    /// Rate limit (requests per second).
    pub rate_limit: Option<u32>,
}
