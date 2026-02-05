//! API key authentication trait and implementation.

use async_trait::async_trait;
use reqwest::RequestBuilder;
use secrecy::{ExposeSecret, SecretString};

use crate::error::Error;

/// Known API key providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiKeyProvider {
    RecallAi,
    AssemblyAi,
}

impl ApiKeyProvider {
    /// Get the provider identifier string.
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiKeyProvider::RecallAi => "recall_ai",
            ApiKeyProvider::AssemblyAi => "assemblyai",
        }
    }
}

/// Authentication method for HTTP requests.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Custom header with optional prefix (e.g., "Authorization: Token xxx")
    ApiKeyHeader {
        header_name: String,
        prefix: Option<String>,
    },
    /// Standard Bearer token
    BearerToken,
    /// HTTP Basic authentication
    BasicAuth { username: String },
}

/// Trait for authenticating HTTP requests with API keys or bearer tokens.
///
/// Implementations handle provider-specific authentication patterns like:
/// - Recall.ai: `Authorization: Token xxx`
/// - AssemblyAI: `authorization: xxx`
#[async_trait]
pub trait ProviderAuth: Send + Sync {
    /// Get the provider identifier.
    fn provider(&self) -> ApiKeyProvider;

    /// Get the authentication method used by this provider.
    fn auth_method(&self) -> AuthMethod;

    /// Apply authentication to a request builder.
    fn authenticate(&self, request: RequestBuilder) -> RequestBuilder;

    /// Verify that the credentials are valid by making a test request.
    /// Returns `true` if credentials are valid, `false` otherwise.
    async fn verify_credentials(&self) -> Result<bool, Error>;
}

/// API key authentication implementation.
///
/// Supports custom header names and prefixes for various provider authentication patterns.
///
/// # Examples
///
/// ```rust,ignore
/// // Recall.ai: Authorization: Token xxx
/// let auth = ApiKeyAuth::new(
///     ApiKeyProvider::RecallAi,
///     SecretString::from("api_key_here"),
///     "Token",
/// );
///
/// // AssemblyAI: authorization: xxx (no prefix)
/// let auth = ApiKeyAuth::new(
///     ApiKeyProvider::AssemblyAi,
///     SecretString::from("api_key_here"),
///     "",
/// );
/// ```
pub struct ApiKeyAuth {
    provider: ApiKeyProvider,
    api_key: SecretString,
    header_name: String,
    prefix: Option<String>,
}

impl ApiKeyAuth {
    /// Create a new API key authenticator.
    ///
    /// # Arguments
    ///
    /// * `provider` - The API provider
    /// * `api_key` - The API key (stored securely)
    /// * `prefix` - Optional prefix for the authorization value (e.g., "Token", "Bearer")
    pub fn new(provider: ApiKeyProvider, api_key: SecretString, prefix: &str) -> Self {
        let (header_name, prefix_opt) = match provider {
            ApiKeyProvider::RecallAi => ("Authorization".to_string(), Some(prefix.to_string())),
            ApiKeyProvider::AssemblyAi => ("authorization".to_string(), None),
        };

        Self {
            provider,
            api_key,
            header_name,
            prefix: prefix_opt.filter(|p| !p.is_empty()),
        }
    }

    /// Get a reference to the API key.
    pub fn api_key(&self) -> &SecretString {
        &self.api_key
    }
}

#[async_trait]
impl ProviderAuth for ApiKeyAuth {
    fn provider(&self) -> ApiKeyProvider {
        self.provider
    }

    fn auth_method(&self) -> AuthMethod {
        AuthMethod::ApiKeyHeader {
            header_name: self.header_name.clone(),
            prefix: self.prefix.clone(),
        }
    }

    fn authenticate(&self, request: RequestBuilder) -> RequestBuilder {
        let auth_value = if let Some(prefix) = &self.prefix {
            format!("{} {}", prefix, self.api_key.expose_secret())
        } else {
            self.api_key.expose_secret().to_string()
        };

        request.header(&self.header_name, auth_value)
    }

    async fn verify_credentials(&self) -> Result<bool, Error> {
        // Provider-specific verification endpoints would be implemented here
        // For now, return Ok(true) as a placeholder
        // In production, this would make an actual API call to verify the key
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_provider_as_str() {
        assert_eq!(ApiKeyProvider::RecallAi.as_str(), "recall_ai");
        assert_eq!(ApiKeyProvider::AssemblyAi.as_str(), "assemblyai");
    }

    #[test]
    fn test_api_key_auth_creation() {
        let api_key = SecretString::from("test_key");
        let auth = ApiKeyAuth::new(ApiKeyProvider::RecallAi, api_key, "Token");

        assert_eq!(auth.provider(), ApiKeyProvider::RecallAi);
        assert_eq!(auth.header_name, "Authorization");
        assert_eq!(auth.prefix, Some("Token".to_string()));
    }

    #[test]
    fn test_assemblyai_auth_no_prefix() {
        let api_key = SecretString::from("test_key");
        let auth = ApiKeyAuth::new(ApiKeyProvider::AssemblyAi, api_key, "");

        assert_eq!(auth.provider(), ApiKeyProvider::AssemblyAi);
        assert_eq!(auth.header_name, "authorization");
        assert_eq!(auth.prefix, None);
    }
}
