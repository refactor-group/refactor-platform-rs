//! Standard Bearer token authentication.

use async_trait::async_trait;
use reqwest::RequestBuilder;
use secrecy::{ExposeSecret, SecretString};

use super::{ApiKeyProvider, AuthMethod, ProviderAuth};
use crate::error::Error;

/// Standard Bearer token authentication.
///
/// Uses the standard `Authorization: Bearer <token>` header pattern.
pub struct BearerTokenAuth {
    provider: ApiKeyProvider,
    token: SecretString,
}

impl BearerTokenAuth {
    /// Create a new Bearer token authenticator.
    pub fn new(provider: ApiKeyProvider, token: SecretString) -> Self {
        Self { provider, token }
    }

    /// Get a reference to the token.
    pub fn token(&self) -> &SecretString {
        &self.token
    }
}

#[async_trait]
impl ProviderAuth for BearerTokenAuth {
    fn provider(&self) -> ApiKeyProvider {
        self.provider
    }

    fn auth_method(&self) -> AuthMethod {
        AuthMethod::BearerToken
    }

    fn authenticate(&self, request: RequestBuilder) -> RequestBuilder {
        request.bearer_auth(self.token.expose_secret())
    }

    async fn verify_credentials(&self) -> Result<bool, Error> {
        // Provider-specific verification would be implemented here
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bearer_token_auth_creation() {
        let token = SecretString::from("test_token".to_string());
        let auth = BearerTokenAuth::new(ApiKeyProvider::RecallAi, token);

        assert_eq!(auth.provider(), ApiKeyProvider::RecallAi);
    }
}
