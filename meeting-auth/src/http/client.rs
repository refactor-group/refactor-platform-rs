//! Authenticated HTTP client builder with middleware.

use std::time::Duration;

use reqwest_middleware::ClientBuilder;
use reqwest_retry::RetryTransientMiddleware;

use super::RetryAfterPolicy;
use crate::api_key::ProviderAuth;

/// HTTP client configuration.
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Request timeout.
    pub timeout: Duration,
    /// Maximum number of retries.
    pub max_retries: u32,
    /// User agent string.
    pub user_agent: String,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            max_retries: 3,
            user_agent: format!("meeting-auth/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Authenticated HTTP client with middleware.
pub type AuthenticatedClient = reqwest_middleware::ClientWithMiddleware;

/// Builder for creating authenticated HTTP clients with middleware.
///
/// Provides a fluent API for constructing HTTP clients with:
/// - Authentication (API keys, bearer tokens)
/// - Retry logic with Retry-After header support
/// - Timeout configuration
/// - Custom middleware
pub struct AuthenticatedClientBuilder {
    config: HttpClientConfig,
    auth: Option<Box<dyn ProviderAuth>>,
}

impl AuthenticatedClientBuilder {
    /// Create a new client builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: HttpClientConfig::default(),
            auth: None,
        }
    }

    /// Set the authentication provider.
    pub fn with_auth(mut self, auth: Box<dyn ProviderAuth>) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Set the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Set the maximum number of retries.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.config.max_retries = max_retries;
        self
    }

    /// Set the user agent string.
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.config.user_agent = user_agent;
        self
    }

    /// Build the configured HTTP client.
    ///
    /// # Returns
    ///
    /// An authenticated HTTP client with middleware configured.
    pub fn build(self) -> Result<AuthenticatedClient, reqwest::Error> {
        // Build the base reqwest client
        let client = reqwest::Client::builder()
            .timeout(self.config.timeout)
            .user_agent(self.config.user_agent)
            .build()?;

        // Add retry middleware with Retry-After support
        let retry_policy = RetryAfterPolicy::new(self.config.max_retries);
        let client_with_middleware = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        Ok(client_with_middleware)
    }
}

impl Default for AuthenticatedClientBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builder_default() {
        let builder = AuthenticatedClientBuilder::new();
        assert_eq!(builder.config.timeout, Duration::from_secs(30));
        assert_eq!(builder.config.max_retries, 3);
    }

    #[test]
    fn test_builder_with_timeout() {
        let builder = AuthenticatedClientBuilder::new().with_timeout(Duration::from_secs(60));
        assert_eq!(builder.config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_builder_with_max_retries() {
        let builder = AuthenticatedClientBuilder::new().with_max_retries(5);
        assert_eq!(builder.config.max_retries, 5);
    }

    #[tokio::test]
    async fn test_build_client() {
        let builder = AuthenticatedClientBuilder::new();
        let result = builder.build();
        assert!(result.is_ok());
    }
}
