//! API key authentication trait and implementation.

use reqwest::RequestBuilder;
use secrecy::{ExposeSecret, SecretString};

/// Known API key providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    RecallAi,
}

impl Provider {
    /// Get the provider identifier string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::RecallAi => "recall_ai",
        }
    }
}

/// Authentication method for HTTP requests.
#[derive(Debug, Clone)]
pub enum AuthMethod {
    /// Custom header with optional prefix (e.g., "Authorization: Token xxx")
    Header {
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
pub trait Authenticate: Send + Sync {
    /// Get the provider identifier.
    fn provider(&self) -> Provider;

    /// Get the authentication method used by this provider.
    fn auth_method(&self) -> AuthMethod;

    /// Apply authentication to a request builder.
    fn authenticate(&self, request: RequestBuilder) -> RequestBuilder;
}

/// API key authentication implementation.
///
/// Supports custom header names and prefixes for various provider authentication patterns.
///
/// # Examples
///
/// ```rust,ignore
/// // Recall.ai: Authorization: Token xxx
/// let auth = Auth::new(
///     Provider::RecallAi,
///     SecretString::from("api_key_here"),
///     "Token",
/// );
/// ```
pub struct Auth {
    provider: Provider,
    api_key: SecretString,
    header_name: String,
    prefix: Option<String>,
}

impl Auth {
    /// Create a new API key authenticator.
    ///
    /// # Arguments
    ///
    /// * `provider` - The API provider
    /// * `api_key` - The API key (stored securely)
    /// * `prefix` - Optional prefix for the authorization value (e.g., "Token", "Bearer")
    pub fn new(provider: Provider, api_key: SecretString, prefix: &str) -> Self {
        let (header_name, prefix_opt) = match provider {
            Provider::RecallAi => ("Authorization".to_string(), Some(prefix.to_string())),
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

impl Authenticate for Auth {
    fn provider(&self) -> Provider {
        self.provider
    }

    fn auth_method(&self) -> AuthMethod {
        AuthMethod::Header {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_provider_as_str() {
        assert_eq!(Provider::RecallAi.as_str(), "recall_ai");
    }

    #[test]
    fn test_api_key_auth_creation() {
        let api_key = SecretString::from("test_key".to_string());
        let auth = Auth::new(Provider::RecallAi, api_key, "Token");

        assert_eq!(auth.provider(), Provider::RecallAi);
        assert_eq!(auth.header_name, "Authorization");
        assert_eq!(auth.prefix, Some("Token".to_string()));
    }
}
