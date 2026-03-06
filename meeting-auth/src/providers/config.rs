//! Pre-configured provider settings.

use crate::api_key::ApiKeyProvider;

/// Provider configuration with endpoints and settings.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Provider identifier.
    pub provider: ApiKeyProvider,
    /// Base API URL.
    pub base_url: String,
    /// Default region (if applicable).
    pub region: Option<String>,
    /// Rate limit (requests per second).
    pub rate_limit: Option<u32>,
}

/// Get Recall.ai configuration.
///
/// # Arguments
///
/// * `region` - AWS region (e.g., "us-west-2")
/// * `base_domain` - Base domain (typically "api.recall.ai")
pub fn recall_ai_config(region: &str, base_domain: &str) -> ProviderConfig {
    ProviderConfig {
        provider: ApiKeyProvider::RecallAi,
        base_url: format!("https://{}/{}", base_domain, region),
        region: Some(region.to_string()),
        rate_limit: Some(10), // Recall.ai has strict rate limits
    }
}

/// Get AssemblyAI configuration.
pub fn assemblyai_config() -> ProviderConfig {
    ProviderConfig {
        provider: ApiKeyProvider::AssemblyAi,
        base_url: "https://api.assemblyai.com".to_string(),
        region: None,
        rate_limit: Some(100),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recall_ai_config() {
        let config = recall_ai_config("us-west-2", "api.recall.ai");
        assert_eq!(config.provider, ApiKeyProvider::RecallAi);
        assert_eq!(config.base_url, "https://api.recall.ai/us-west-2");
        assert_eq!(config.region, Some("us-west-2".to_string()));
    }

    #[test]
    fn test_assemblyai_config() {
        let config = assemblyai_config();
        assert_eq!(config.provider, ApiKeyProvider::AssemblyAi);
        assert_eq!(config.base_url, "https://api.assemblyai.com");
        assert_eq!(config.region, None);
    }
}
